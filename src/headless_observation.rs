#![cfg(all(feature = "headless-http", feature = "servo-runtime"))]

use serde::{Deserialize, Serialize};

use crate::{agent::AgentSurface, engine::EngineError};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpatialRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpatialElement {
    pub id: String,
    pub selector: String,
    pub role: String,
    pub text: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub href: Option<String>,
    pub bbox: SpatialRect,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpatialPageObservation {
    pub url: String,
    pub title: String,
    pub viewport_width: f64,
    pub viewport_height: f64,
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub scroll_height: f64,
    pub elements: Vec<SpatialElement>,
}

impl AgentSurface {
    /// Return a spatially grounded observation suitable for Engine C without
    /// taking a screenshot or depending on a remote browser protocol.
    ///
    /// This reads layout geometry from the same live Servo document that the
    /// Spatial Browser is rendering. The selector returned for an element is
    /// also directly usable by AgentSurface click/type/submit actions.
    pub async fn observe_spatial(&self) -> Result<SpatialPageObservation, EngineError> {
        self.eval(SPATIAL_OBSERVE_SCRIPT).await
    }
}

const SPATIAL_OBSERVE_SCRIPT: &str = r##"(() => {
  const esc = (value) => (window.CSS && CSS.escape)
    ? CSS.escape(value)
    : String(value).replace(/[^A-Za-z0-9_-]/g, c => "\\" + c);

  const selector = (el) => {
    if (el.id) return "#" + esc(el.id);
    const name = el.getAttribute("name");
    if (name) return el.tagName.toLowerCase() + "[name=" + JSON.stringify(name) + "]";
    const aria = el.getAttribute("aria-label");
    if (aria) return el.tagName.toLowerCase() + "[aria-label=" + JSON.stringify(aria) + "]";
    const parent = el.parentElement;
    if (!parent) return el.tagName.toLowerCase();
    const siblings = Array.from(parent.children).filter(x => x.tagName === el.tagName);
    const index = siblings.indexOf(el) + 1;
    const prefix = parent.id ? ("#" + esc(parent.id) + " > ") : (parent.tagName.toLowerCase() + " > ");
    return prefix + el.tagName.toLowerCase() + ":nth-of-type(" + index + ")";
  };

  const text = (el) => (
    el.getAttribute("aria-label") ||
    el.getAttribute("placeholder") ||
    el.getAttribute("title") ||
    el.getAttribute("alt") ||
    el.innerText ||
    el.value ||
    ""
  ).replace(/\s+/g, " ").trim().slice(0, 32768);

  const visible = (el) => {
    const r = el.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) return false;
    const s = window.getComputedStyle(el);
    return s.visibility !== "hidden" && s.display !== "none" && Number(s.opacity || 1) !== 0;
  };

  // Include both interactive controls and common content-bearing elements.
  // Engine C can then associate labels, prices, values and controls spatially.
  const q = [
    "a[href]", "button", "input", "select", "textarea",
    "[role=button]", "[role=link]", "[role=textbox]", "[role=checkbox]",
    "[contenteditable=true]", "label", "h1", "h2", "h3", "h4",
    "p", "li", "td", "th", "dt", "dd", "article", "section",
    "[data-testid]", "[aria-label]"
  ].join(",");

  const nodes = Array.from(document.querySelectorAll(q))
    .filter(visible)
    .slice(0, 20000);

  const elements = nodes.map((el, index) => {
    const r = el.getBoundingClientRect();
    const s = selector(el);
    return {
      id: s + "@" + index,
      selector: s,
      role: el.getAttribute("role") || el.tagName.toLowerCase(),
      text: text(el),
      value: (el.value === undefined || el.value === null) ? null : String(el.value).slice(0, 4096),
      href: el.getAttribute("href"),
      bbox: {
        x: Math.max(0, r.left),
        y: Math.max(0, r.top),
        width: r.width,
        height: r.height
      },
      disabled: !!el.disabled
    };
  });

  return {
    url: location.href,
    title: document.title || "",
    viewport_width: window.innerWidth,
    viewport_height: window.innerHeight,
    scroll_x: window.scrollX,
    scroll_y: window.scrollY,
    scroll_height: Math.max(document.documentElement.scrollHeight, document.body ? document.body.scrollHeight : 0),
    elements
  };
})()"##;
