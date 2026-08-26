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
}

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

  const elements = Array.from(document.querySelectorAll(q))
    .filter(visible)
    .slice(0, 250)
    .map(el => ({
      selector: sel(el),
      role: el.getAttribute("role") || el.tagName.toLowerCase(),
      name: named(el),
      value: (el.value === undefined || el.value === null) ? null : String(el.value).slice(0, 200),
      href: el.getAttribute("href"),
      disabled: !!el.disabled
    }));

  return {
    url: location.href,
    title: document.title || "",
    text: (document.body ? document.body.innerText : "")
            .replace(/\s+/g, " ").trim().slice(0, 40000),
    elements: elements,
    scroll_y: window.scrollY,
    scroll_height: document.documentElement.scrollHeight
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
