//! Live semantic tree of the page, from the engine rather than from script.
//!
//! NEROA_SEMANTIC_TREE_V9
//!
//! Servo emits AccessKit tree updates for every document it renders. That is a
//! standardised structure of roles, labels, values and hierarchy - the same
//! thing a screen reader consumes - maintained by the engine as part of
//! layout.
//!
//! Three reasons this beats scraping the DOM from injected JavaScript, which
//! is what the agent surface currently does:
//!
//!  * It is computed by the engine, so it reflects what the page actually
//!    presents rather than what a query selector happens to match.
//!  * It carries accessibility semantics - a div wired up as a button with
//!    aria-role and aria-label appears as a button with a name.
//!  * A page cannot defeat it by obfuscating markup, because the same tree
//!    is what assistive technology relies on.
//!
//! Updates are incremental: each one carries only changed nodes, and a node
//! must be re-sent whole when any field changes. The store applies them in
//! order and keeps the resolved tree.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// One node of the resolved semantic tree.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SemanticNode {
    pub id: u64,

    /// AccessKit role, e.g. `Button`, `Link`, `TextInput`.
    pub role: String,

    /// Accessible name.
    #[serde(default)]
    pub label: Option<String>,

    #[serde(default)]
    pub value: Option<String>,

    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub children: Vec<u64>,
}

impl SemanticNode {
    /// Whether this node is something an agent can act on.
    pub fn is_actionable(&self) -> bool {
        matches!(
            self.role.as_str(),
            "Button"
                | "Link"
                | "CheckBox"
                | "RadioButton"
                | "ComboBox"
                | "TextInput"
                | "MultilineTextInput"
                | "SearchInput"
                | "EmailInput"
                | "PasswordInput"
                | "NumberInput"
                | "TelephoneInput"
                | "UrlInput"
                | "ListBox"
                | "Switch"
                | "Tab"
                | "MenuItem"
                | "Slider"
        )
    }

    /// Structural landmark: the parts of a page a reader navigates by.
    ///
    /// NEROA_SEMANTIC_ROLES_V10: Servo 0.5 emits structure and text but no
    /// interactive roles - a link arrives as GenericContainer, not Link. The
    /// tree is therefore excellent for comprehension and useless for deciding
    /// what to click, which is why the agent surface keeps its DOM query path
    /// for actions and uses this for understanding.
    pub fn is_landmark(&self) -> bool {
        matches!(
            self.role.as_str(),
            "RootWebArea"
                | "Main"
                | "Navigation"
                | "Banner"
                | "ContentInfo"
                | "Heading"
                | "Article"
                | "Region"
                | "Form"
                | "Search"
                | "List"
                | "Table"
        )
    }

    /// Whether this node's text is page content rather than inline code.
    ///
    /// Servo puts the contents of script and style elements into the tree as
    /// TextRun, so an unfiltered outline is mostly minified JavaScript. This
    /// is a heuristic, not a guarantee: it trades a little real text for a lot
    /// less noise, which is the right trade when the output goes in a prompt.
    pub fn is_readable_text(&self) -> bool {
        let Some(text) = self.name() else {
            return false;
        };

        if text.len() > 400 {
            return false;
        }

        let head: String = text.chars().take(80).collect();

        let codey = head.starts_with("(function")
            || head.starts_with("window.")
            || head.starts_with("(RLQ")
            || head.contains("){")
            || head.contains(";}")
            || head.matches('{').count() + head.matches(';').count() >= 3;

        !codey
    }

    /// Best available human-readable name.
    pub fn name(&self) -> Option<&str> {
        self.label
            .as_deref()
            .or(self.value.as_deref())
            .or(self.description.as_deref())
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }
}

/// One AccessKit tree: the webview shell, or a document grafted into it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SemanticDocument {
    pub nodes: HashMap<u64, SemanticNode>,

    #[serde(default)]
    pub root: Option<u64>,

    #[serde(default)]
    pub focus: Option<u64>,
}

/// Semantic state for one view.
///
/// NEROA_SEMANTIC_TREE_V9: Servo emits several trees, not one. The webview
/// shell is its own small tree (a ScrollView wrapping a graft node), and each
/// document is grafted in as a separate tree with its own id. Node ids are
/// only unique *within* a tree - the shell and a document both number from
/// zero - so flattening them into a single map silently corrupts both.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SemanticTree {
    /// Keyed by tree id.
    pub documents: HashMap<String, SemanticDocument>,

    /// The document tree currently worth reading: the most recently updated
    /// one that is not the shell.
    #[serde(default)]
    pub active: Option<String>,

    /// Number of updates applied. Lets a caller tell a stale read from a
    /// genuinely unchanged page.
    #[serde(default)]
    pub generation: u64,
}

impl SemanticTree {
    /// The document an agent should read.
    pub fn document(&self) -> Option<&SemanticDocument> {
        self.active
            .as_ref()
            .and_then(|id| self.documents.get(id))
            .or_else(|| {
                // Fall back to the largest tree rather than returning nothing:
                // a missing observation is worse than an imperfect one.
                self.documents.values().max_by_key(|doc| doc.nodes.len())
            })
    }

    pub fn node_count(&self) -> usize {
        self.document().map(|doc| doc.nodes.len()).unwrap_or(0)
    }

    /// Depth-first walk from the active document's root, yielding (depth, node).
    ///
    /// Guards against cycles: a malformed or mid-update tree can contain one,
    /// and an agent observation is not worth hanging the host thread over.
    pub fn walk(&self) -> Vec<(usize, &SemanticNode)> {
        let mut out = Vec::new();

        let Some(doc) = self.document() else {
            return out;
        };

        let Some(root) = doc.root.and_then(|id| doc.nodes.get(&id)) else {
            return out;
        };

        let mut seen = std::collections::HashSet::new();

        let mut stack = vec![(0usize, root)];

        while let Some((depth, node)) = stack.pop() {
            if !seen.insert(node.id) {
                continue;
            }

            out.push((depth, node));

            for child in node.children.iter().rev() {
                if let Some(child) = doc.nodes.get(child) {
                    stack.push((depth + 1, child));
                }
            }
        }

        out
    }

    /// Structural landmarks in document order: the page's shape.
    pub fn landmarks(&self) -> Vec<&SemanticNode> {
        self.walk()
            .into_iter()
            .map(|(_, node)| node)
            .filter(|node| node.is_landmark())
            .collect()
    }

    /// Everything an agent could act on, in document order.
    pub fn actionable(&self) -> Vec<&SemanticNode> {
        self.walk()
            .into_iter()
            .map(|(_, node)| node)
            .filter(|node| node.is_actionable())
            .collect()
    }

    /// Compact indented rendering, suitable for putting in a prompt.
    ///
    /// Nodes with no name and no actionable role are skipped: they are
    /// structural noise that costs tokens and tells a model nothing.
    pub fn outline(&self, max_lines: usize) -> String {
        let mut out = String::new();

        let mut lines = 0usize;

        for (depth, node) in self.walk() {
            if lines >= max_lines {
                break;
            }

            let indent = "  ".repeat(depth.min(12));

            if node.role == "TextRun" && !node.is_readable_text() {
                continue;
            }

            match node.name() {
                Some(name) => {
                    out.push_str(&indent);
                    out.push_str(&node.role);
                    out.push_str(": ");
                    out.push_str(&name.chars().take(160).collect::<String>());
                    out.push('\n');

                    lines += 1;
                }

                None if node.is_actionable() => {
                    out.push_str(&indent);
                    out.push_str(&node.role);
                    out.push('\n');

                    lines += 1;
                }

                None => {}
            }
        }

        out
    }
}

/// Shared store the delegate writes and the agent reads.
pub type SharedSemanticTree = Arc<Mutex<SemanticTree>>;

/// Apply one AccessKit update to the store.
pub fn apply_update(store: &SharedSemanticTree, update: &servo::accesskit::TreeUpdate) {
    let Ok(mut tree) = store.lock() else {
        return;
    };

    let key = format!("{:?}", update.tree_id);

    let doc = tree.documents.entry(key.clone()).or_default();

    for (id, node) in &update.nodes {
        doc.nodes.insert(
            id.0,
            SemanticNode {
                id: id.0,
                role: format!("{:?}", node.role()),
                label: node.label().map(str::to_string),
                value: node.value().map(str::to_string),
                description: node.description().map(str::to_string),
                children: node.children().iter().map(|child| child.0).collect(),
            },
        );
    }

    if let Some(root) = update.tree.as_ref() {
        doc.root = Some(root.root.0);
    }

    doc.focus = Some(update.focus.0);

    let root_role = doc
        .root
        .and_then(|id| doc.nodes.get(&id))
        .map(|node| node.role.clone())
        .unwrap_or_default();

    let node_count = doc.nodes.len();

    // The shell tree is a ScrollView with a graft child and nothing else.
    // Anything larger, or rooted differently, is a real document.
    let is_shell = root_role == "ScrollView" && node_count <= 3;

    if !is_shell {
        tree.active = Some(key);
    }

    tree.generation = tree.generation.saturating_add(1);
}
