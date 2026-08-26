//! What the agent sees, and the scripts that produce it.
//!
//! NEROA_AGENT_SURFACE_V7

use serde::{Deserialize, Serialize};

/// One actionable element on the page.
///
/// `selector` is the important field: it is a real CSS selector the agent can
/// pass straight back to click() or type_text(). Screenshot-driven automation
/// has to convert a description into coordinates and hope; here the thing the
/// model is shown is the thing it can act on.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ElementHandle {
    pub selector: String,

    /// ARIA role where present, otherwise the tag name.
    pub role: String,

    /// Accessible name: label, aria-label, placeholder, or trimmed text.
    pub name: String,

    #[serde(default)]
    pub value: Option<String>,

    #[serde(default)]
    pub href: Option<String>,

    #[serde(default)]
    pub disabled: bool,

    // NEROA_QC_OBSERVATION_V18
    //
    // The nearest ancestor that would receive a delegated event for this
    // control, or null if the control catches its own. This is what finds the
    // Code Fabric bug - 62 controls with zero listeners on themselves, routed
    // by one delegated listener on a container whose module never loaded - by
    // walking the ancestor chain instead of a useless per-element count.
    #[serde(default)]
    pub delegated_from: Option<String>,

    /// Three states, deliberately:
    ///   Some(true)  - a handler on this control or an ancestor would catch it
    ///   Some(false) - checked, and nothing in the chain would: a finding
    ///   None        - could not determine (instrumentation not active)
    ///
    /// false must never mean "couldn't tell": a QC tool that cries wolf on
    /// working buttons gets turned off, and then it catches nothing.
    #[serde(default)]
    pub reachable: Option<bool>,
}

/// A container repainted by more than one writer. NEROA_QC_OBSERVATION_V18:
/// two writers to one mount slot becomes a fact instead of an inference from
/// mutation counts - the audit painted over by the error state, and the shared
/// slot before it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepaintRecord {
    pub selector: String,
    /// Distinct writers seen mutating this container's subtree.
    #[serde(deserialize_with = "de_count")]
    pub writers: u32,
    /// Total mutations observed on it.
    #[serde(deserialize_with = "de_count")]
    pub mutations: u32,
    /// Milliseconds since instrumentation start of the most recent mutation.
    pub last_at_ms: f64,
}

/// Servo returns every JS number as an f64, so an integer count arrives as
/// `2.0`. Coerce it back to a u32 so the field stays an integer for readers.
fn de_count<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = f64::deserialize(deserializer)?;
    Ok(value.max(0.0).round() as u32)
}

/// A console error, timestamped with source and stack. NEROA_QC_OBSERVATION_V18.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsoleError {
    pub message: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub stack: Option<String>,
    pub at_ms: f64,
}

/// A page as an agent reads it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PageObservation {
    pub url: String,

    pub title: String,

    /// Collapsed visible text, truncated.
    pub text: String,

    pub elements: Vec<ElementHandle>,

    #[serde(default)]
    pub scroll_y: f32,

    #[serde(default)]
    pub scroll_height: f32,

    // NEROA_QC_OBSERVATION_V18: all optional with defaults, so anything
    // reading PageObservation today keeps working untouched.

    /// True when the QC instrumentation user script was active for this page.
    /// When false, reachability is undetermined rather than a confident false.
    #[serde(default)]
    pub qc_active: bool,

    /// Classes present in the DOM that matched no rule in any loaded
    /// stylesheet. The grey-button bug stated as its cause: fav-card, fav-h,
    /// fav-chip - used, styled nowhere - found on load instead of by grepping.
    #[serde(default)]
    pub unmatched_classes: Vec<String>,

    #[serde(default)]
    pub repaints: Vec<RepaintRecord>,

    #[serde(default)]
    pub console_errors: Vec<ConsoleError>,
}

/// Document-start instrumentation. NEROA_QC_OBSERVATION_V18.
///
/// Injected as a Servo user script so it runs at head parse, before app
/// listeners attach. It wraps addEventListener to record which elements carry
/// which listeners - the only way to see delegated listeners, which are not
/// reflected in the DOM - and records mutations and console errors. observe()
/// reads what this collected; without it, reachability is undetermined, never
/// a false positive.
/// The QC instrumentation user script. NEROA_QC_OBSERVATION_V18.
pub fn qc_instrumentation_script() -> &'static str {
    QC_INSTRUMENTATION_SCRIPT
}

pub(crate) const QC_INSTRUMENTATION_SCRIPT: &str = r##"(() => {
  if (window.__neroaQC) { return; }
  const QC = window.__neroaQC = {
    active: true,
    t0: (performance && performance.now) ? performance.now() : 0,
    listeners: new WeakMap(),
    writerSeq: 0,
    repaints: new Map(),
    errors: [],
  };
  const now = () => (((performance && performance.now) ? performance.now() : 0) - QC.t0);

  // Record listener registrations. Delegated listeners live on containers and
  // are invisible to the DOM; this is the only place they can be seen.
  const add = EventTarget.prototype.addEventListener;
  EventTarget.prototype.addEventListener = function (type, listener, opts) {
    try {
      if (this instanceof Element) {
        let set = QC.listeners.get(this);
        if (!set) { set = new Set(); QC.listeners.set(this, set); }
        set.add(type);
      }
    } catch (e) {}
    return add.call(this, type, listener, opts);
  };

  // Repaint tracking: attribute each container's mutations to a writer. We
  // cannot see the calling module, but a distinct microtask-stamped writer per
  // synchronous burst turns "two writers to one container" into a fact.
  try {
    const mo = new MutationObserver((records) => {
      QC.writerSeq++;
      for (const r of records) {
        const el = r.target instanceof Element ? r.target
                 : (r.target && r.target.parentElement);
        if (!el) { continue; }
        let rec = QC.repaints.get(el);
        if (!rec) { rec = { writers: new Set(), mutations: 0, last: 0 }; QC.repaints.set(el, rec); }
        rec.writers.add(QC.writerSeq);
        rec.mutations++;
        rec.last = now();
      }
    });
    const start = () => mo.observe(document.documentElement || document, {
      childList: true, subtree: true, attributes: true, characterData: true,
    });
    if (document.documentElement) { start(); }
    else { document.addEventListener("readystatechange", start, { once: true }); }
  } catch (e) {}

  // Console errors, timestamped with source and stack.
  const err = console.error;
  console.error = function (...args) {
    try {
      QC.errors.push({
        message: args.map(a => (a && a.message) ? a.message : String(a)).join(" ").slice(0, 2000),
        stack: (args.find(a => a && a.stack) || {}).stack || null,
        at: now(),
      });
    } catch (e) {}
    return err.apply(this, args);
  };
  window.addEventListener("error", (e) => {
    try {
      QC.errors.push({
        message: String(e.message || "error"),
        source: e.filename || null,
        stack: (e.error && e.error.stack) || null,
        at: now(),
      });
    } catch (x) {}
  });
})()"##;

/// Build a stable-ish selector for an element, preferring id, then a
/// name/aria-label attribute, then nth-of-type within the parent. Generated
/// class soups are deliberately avoided: they change between deploys and make
/// recorded traces worthless for training.
pub(crate) const OBSERVE_SCRIPT: &str = r##"(() => {
  const sel = (el) => {
    if (el.id) { return "#" + CSS.escape(el.id); }
    const nm = el.getAttribute("name");
    if (nm) { return el.tagName.toLowerCase() + "[name=" + JSON.stringify(nm) + "]"; }
    const al = el.getAttribute("aria-label");
    if (al) { return el.tagName.toLowerCase() + "[aria-label=" + JSON.stringify(al) + "]"; }
    const parent = el.parentElement;
    if (!parent) { return el.tagName.toLowerCase(); }
    const tag = el.tagName;
    const sibs = Array.from(parent.children).filter(c => c.tagName === tag);
    const idx = sibs.indexOf(el) + 1;
    const base = parent.id
      ? "#" + CSS.escape(parent.id) + " > "
      : (parent.tagName.toLowerCase() + " > ");
    return base + tag.toLowerCase() + ":nth-of-type(" + idx + ")";
  };

  const named = (el) =>
    (el.getAttribute("aria-label")
      || el.getAttribute("placeholder")
      || el.getAttribute("title")
      || el.getAttribute("alt")
      || (el.labels && el.labels[0] && el.labels[0].innerText)
      || el.innerText
      || el.value
      || "").replace(/\s+/g, " ").trim().slice(0, 160);

  const visible = (el) => {
    const r = el.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) { return false; }
    const s = window.getComputedStyle(el);
    return s.visibility !== "hidden" && s.display !== "none";
  };

  const q = "a[href], button, input, select, textarea, [role=button],"
          + "[role=link], [role=textbox], [role=checkbox], [contenteditable=true]";

  // NEROA_QC_OBSERVATION_V18: reachability from the instrumentation, if active.
  const QC = window.__neroaQC;
  const active = !!(QC && QC.active);

  // A control's own reachability: an inline handler, an interactive default, or
  // a recorded listener on itself.
  const CLICKISH = new Set(["click", "mousedown", "mouseup", "pointerdown", "pointerup", "keydown", "keyup"]);
  const hasOwn = (el) => {
    // An inline handler, or a recorded addEventListener, is a real handler.
    if (el.onclick || el.getAttribute("onclick")) { return true; }
    if (active && QC.listeners.has(el)) {
      for (const t of QC.listeners.get(el)) { if (CLICKISH.has(t)) { return true; } }
    }
    const tag = el.tagName;
    // Native interactivity that does something without a script handler.
    if (tag === "A" && el.getAttribute("href")) { return true; }
    if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") { return true; }
    // A submit/reset button in a form acts natively. A bare <button> does
    // NOT: it looks interactive and does nothing, which is exactly the
    // Code Fabric defect - so it must fall through to the delegation walk,
    // not be assumed reachable because it is a button.
    if (tag === "BUTTON") {
      const type = (el.getAttribute("type") || "submit").toLowerCase();
      if ((type === "submit" || type === "reset") && el.form) { return true; }
    }
    return false;
  };

  // Walk up from a control: the nearest ancestor (or self) that would catch a
  // delegated event, and whether anything in the chain would.
  const reach = (el) => {
    if (hasOwn(el)) { return { delegated_from: null, reachable: active ? true : null }; }
    let node = el.parentElement;
    while (node) {
      if (hasOwn(node)) { return { delegated_from: sel(node), reachable: true }; }
      node = node.parentElement;
    }
    // Nothing caught it. Only assert false when instrumentation was active,
    // so a false is always "checked", never "couldn't determine".
    return { delegated_from: null, reachable: active ? false : null };
  };

  const elements = Array.from(document.querySelectorAll(q))
    .filter(visible)
    .slice(0, 250)
    .map(el => {
      const r = reach(el);
      return {
        selector: sel(el),
        role: el.getAttribute("role") || el.tagName.toLowerCase(),
        name: named(el),
        value: (el.value === undefined || el.value === null) ? null : String(el.value).slice(0, 200),
        href: el.getAttribute("href"),
        disabled: !!el.disabled,
        delegated_from: r.delegated_from,
        reachable: r.reachable,
      };
    });

  // Classes present in the DOM that matched no rule in any loaded stylesheet.
  const styledClasses = new Set();
  try {
    for (const sheet of Array.from(document.styleSheets)) {
      let rules;
      try { rules = sheet.cssRules; } catch (e) { continue; } // cross-origin
      if (!rules) { continue; }
      for (const rule of Array.from(rules)) {
        const text = rule.selectorText;
        if (!text) { continue; }
        const m = text.match(/\.[A-Za-z0-9_-]+/g);
        if (m) { for (const c of m) { styledClasses.add(c.slice(1)); } }
      }
    }
  } catch (e) {}
  const domClasses = new Set();
  for (const el of Array.from(document.querySelectorAll("[class]")).slice(0, 20000)) {
    for (const c of el.classList) { domClasses.add(c); }
  }
  const unmatched = [];
  for (const c of domClasses) { if (!styledClasses.has(c)) { unmatched.push(c); } }

  // Repaints and console errors from the instrumentation.
  const repaints = [];
  const errors = [];
  if (active) {
    for (const [el, rec] of QC.repaints) {
      if (rec.writers.size >= 2) {
        repaints.push({ selector: sel(el), writers: rec.writers.size, mutations: rec.mutations, last_at_ms: rec.last });
      }
    }
    repaints.sort((a, b) => b.writers - a.writers).splice(50);
    for (const e of QC.errors.slice(-100)) {
      errors.push({ message: e.message, source: e.source || null, stack: e.stack || null, at_ms: e.at });
    }
  }

  return {
    url: location.href,
    title: document.title || "",
    text: (document.body ? document.body.innerText : "")
            .replace(/\s+/g, " ").trim().slice(0, 40000),
    elements: elements,
    scroll_y: window.scrollY,
    scroll_height: document.documentElement.scrollHeight,
    qc_active: active,
    unmatched_classes: unmatched.slice(0, 500),
    repaints: repaints,
    console_errors: errors,
  };
})()"##;

pub(crate) const READ_TEXT_SCRIPT: &str = r#"(() => (document.body ? document.body.innerText : "")
    .replace(/\s+/g, " ").trim().slice(0, 200000))()"#;

pub(crate) const ACT_SCRIPT: &str = r#"(() => {
  const el = document.querySelector(__SELECTOR__);
  if (!el) { return false; }
  __BODY__
  return true;
})()"#;

pub(crate) const CLICK_BODY: &str =
    r#"el.scrollIntoView({block:"center"}); el.click();"#;

pub(crate) const TYPE_BODY: &str = r#"el.focus();
  const d = Object.getOwnPropertyDescriptor(el.constructor.prototype, "value");
  if (d && d.set) { d.set.call(el, __VALUE__); } else { el.value = __VALUE__; }
  el.dispatchEvent(new Event("input", {bubbles:true}));
  el.dispatchEvent(new Event("change", {bubbles:true}));"#;

pub(crate) const SUBMIT_BODY: &str = r#"const form = el.form || el.closest("form") || el;
  if (form) { if (form.requestSubmit) { form.requestSubmit(); }
              else if (form.submit) { form.submit(); } }"#;

pub(crate) const EXTRACT_SCRIPT: &str = r#"(() => {
  const spec = __SPEC__;
  return Array.from(document.querySelectorAll(__ROOT__)).map(root => {
    const row = {};
    for (const key of Object.keys(spec)) {
      const s = spec[key];
      const hit = s === "." ? root : root.querySelector(s);
      row[key] = hit ? (hit.getAttribute("href") || hit.innerText || "").trim() : null;
    }
    return row;
  });
})()"#;
