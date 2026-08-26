//! Recognising the browser-based code IDEs.
//!
//! NEROA_CODE_IDE_V19
//!
//! The Spatial Browser is a sandbox where people come to code, in whatever
//! web IDE they prefer, and what they build must be auditable. Recognising
//! which IDE is loaded is the first step: it tags every code receipt with the
//! surface the work happened on, and it tells the capture layer which
//! save/run gestures to expect.
//!
//! Recognition is by host only - deliberately coarse and robust. An IDE's DOM
//! changes every deploy; its hostname does not. The semantic capture that sits
//! on top (a save, a run) is driven by near-universal keyboard gestures rather
//! than per-app DOM scraping, so it does not rot when an IDE ships a redesign.

use serde::{Deserialize, Serialize};

/// The editor family, which decides how much can be read from the page beyond
/// the keyboard gestures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditorFamily {
    /// Monaco / VS Code shell - vscode.dev, github.dev, Gitpod. The active
    /// file usually shows in the document title.
    Monaco,
    /// CodeMirror-based - Replit and others.
    CodeMirror,
    /// A bespoke editor - StackBlitz, CodeSandbox.
    Other,
}

/// What kind of work surface this is. NEROA_CODE_IDE_V19 / AI workstations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SurfaceKind {
    /// A browser-based code editor.
    CodeIde,
    /// An AI workstation - a place where work is done with a model:
    /// notebooks, prompt-to-app builders, agent surfaces.
    AiWorkstation,
}

/// One recognised work surface - a code IDE or an AI workstation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeIde {
    /// Stable, boring identifier used in receipt evidence, e.g. "vscode-web".
    pub id: String,
    /// Human name.
    pub name: String,
    pub family: EditorFamily,
    pub kind: SurfaceKind,
}

/// Recognise the IDE for a URL, by host. Returns None for an ordinary page.
pub fn detect_code_ide(url: &str) -> Option<CodeIde> {
    let host = host_of(url)?;

    // Match on the registrable-ish suffix so subdomains (a StackBlitz project
    // on projectid.stackblitz.io, a Gitpod workspace) still resolve.
    let ide = |id: &str, name: &str, family, kind| {
        Some(CodeIde {
            id: id.to_string(),
            name: name.to_string(),
            family,
            kind,
        })
    };

    use EditorFamily::*;

    // Code IDEs.
    if host == "vscode.dev" || host.ends_with(".vscode.dev") {
        return ide("vscode-web", "VS Code for Web", Monaco, SurfaceKind::CodeIde);
    }
    if host == "github.dev" || host.ends_with(".github.dev") {
        return ide("github-dev", "github.dev", Monaco, SurfaceKind::CodeIde);
    }
    if host.ends_with("stackblitz.com") || host.ends_with("stackblitz.io") {
        return ide("stackblitz", "StackBlitz", Other, SurfaceKind::CodeIde);
    }
    if host.ends_with("codesandbox.io") || host.ends_with("csb.app") {
        return ide("codesandbox", "CodeSandbox", Other, SurfaceKind::CodeIde);
    }
    if host.ends_with("gitpod.io") {
        return ide("gitpod", "Gitpod", Monaco, SurfaceKind::CodeIde);
    }
    if host.ends_with("replit.com") || host.ends_with("repl.co") || host.ends_with("replit.dev") {
        return ide("replit", "Replit", CodeMirror, SurfaceKind::CodeIde);
    }
    if host.ends_with("glitch.com") || host.ends_with("glitch.me") {
        return ide("glitch", "Glitch", CodeMirror, SurfaceKind::CodeIde);
    }
    if host.ends_with("codeanywhere.com") {
        return ide("codeanywhere", "Codeanywhere", Other, SurfaceKind::CodeIde);
    }

    // AI workstations - notebooks, prompt-to-app builders, agent surfaces.
    // Work done here is real work and belongs on the ledger like any other.
    if host == "colab.research.google.com" {
        return ide("colab", "Google Colab", Other, SurfaceKind::AiWorkstation);
    }
    if host.ends_with("kaggle.com") {
        return ide("kaggle", "Kaggle", Other, SurfaceKind::AiWorkstation);
    }
    if host.ends_with("huggingface.co") {
        return ide("huggingface", "Hugging Face", Other, SurfaceKind::AiWorkstation);
    }
    if host.ends_with("jupyter.org") || host.contains("jupyter") {
        return ide("jupyter", "Jupyter", CodeMirror, SurfaceKind::AiWorkstation);
    }
    if host == "bolt.new" || host.ends_with(".bolt.new") {
        return ide("bolt", "Bolt", Other, SurfaceKind::AiWorkstation);
    }
    if host == "v0.dev" || host.ends_with(".v0.dev") {
        return ide("v0", "v0", Other, SurfaceKind::AiWorkstation);
    }
    if host.ends_with("lovable.dev") || host.ends_with("lovable.app") {
        return ide("lovable", "Lovable", Other, SurfaceKind::AiWorkstation);
    }
    if host.ends_with("cursor.com") || host.ends_with("cursor.sh") {
        return ide("cursor", "Cursor", Monaco, SurfaceKind::AiWorkstation);
    }
    if host == "chatgpt.com" || host.ends_with(".chatgpt.com") || host == "chat.openai.com" {
        return ide("chatgpt", "ChatGPT", Other, SurfaceKind::AiWorkstation);
    }
    if host == "claude.ai" || host.ends_with(".claude.ai") {
        return ide("claude", "Claude", Other, SurfaceKind::AiWorkstation);
    }
    if host.ends_with("perplexity.ai") {
        return ide("perplexity", "Perplexity", Other, SurfaceKind::AiWorkstation);
    }

    None
}

fn host_of(url: &str) -> Option<String> {
    let without_scheme = url.split("://").nth(1).unwrap_or(url);
    let authority = without_scheme.split('/').next().unwrap_or("");
    let host = authority.split('@').last().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host);

    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

/// The starting points the sandbox offers - the big code browsers, ready to
/// open. Names and URLs only; the browser navigates to them like anything else.
pub fn launch_presets() -> &'static [(&'static str, &'static str)] {
    &[
        ("VS Code for Web", "https://vscode.dev"),
        ("github.dev", "https://github.dev"),
        ("StackBlitz", "https://stackblitz.com"),
        ("CodeSandbox", "https://codesandbox.io"),
        ("Gitpod", "https://gitpod.io"),
        ("Replit", "https://replit.com"),
        ("Glitch", "https://glitch.com"),
        // AI workstations.
        ("Google Colab", "https://colab.research.google.com"),
        ("Hugging Face", "https://huggingface.co"),
        ("Kaggle", "https://kaggle.com/code"),
        ("Bolt", "https://bolt.new"),
        ("v0", "https://v0.dev"),
        ("Lovable", "https://lovable.dev"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_the_big_code_browsers() {
        assert_eq!(detect_code_ide("https://vscode.dev/").unwrap().id, "vscode-web");
        assert_eq!(detect_code_ide("https://github.dev/user/repo").unwrap().id, "github-dev");
        assert_eq!(
            detect_code_ide("https://abc123.stackblitz.io/").unwrap().id,
            "stackblitz"
        );
        assert_eq!(detect_code_ide("https://codesandbox.io/s/xyz").unwrap().id, "codesandbox");
        assert_eq!(detect_code_ide("https://user.gitpod.io/").unwrap().id, "gitpod");
        assert_eq!(detect_code_ide("https://replit.com/@user/proj").unwrap().id, "replit");
    }

    #[test]
    fn family_is_carried_through() {
        assert_eq!(detect_code_ide("https://vscode.dev").unwrap().family, EditorFamily::Monaco);
        assert_eq!(detect_code_ide("https://replit.com").unwrap().family, EditorFamily::CodeMirror);
    }

    #[test]
    fn recognises_ai_workstations() {
        let colab = detect_code_ide("https://colab.research.google.com/drive/abc").unwrap();
        assert_eq!(colab.id, "colab");
        assert_eq!(colab.kind, SurfaceKind::AiWorkstation);

        assert_eq!(detect_code_ide("https://bolt.new/~/x").unwrap().id, "bolt");
        assert_eq!(detect_code_ide("https://v0.dev/chat/x").unwrap().id, "v0");

        // A code IDE stays a code IDE.
        assert_eq!(detect_code_ide("https://vscode.dev").unwrap().kind, SurfaceKind::CodeIde);
    }

    #[test]
    fn an_ordinary_page_is_not_an_ide() {
        assert!(detect_code_ide("https://example.com").is_none());
        assert!(detect_code_ide("https://news.ycombinator.com").is_none());
    }
}
