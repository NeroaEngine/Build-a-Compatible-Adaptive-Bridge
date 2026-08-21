use crate::types::{EngineCapabilities, PageSignals, RouteTarget};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteDecision {
    pub target: RouteTarget,
    pub reasons: Vec<String>,
}

impl RouteDecision {
    pub fn new(target: RouteTarget, reason: impl Into<String>) -> Self {
        Self {
            target,
            reasons: vec![reason.into()],
        }
    }

    pub fn push_reason(mut self, reason: impl Into<String>) -> Self {
        self.reasons.push(reason.into());
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteRule {
    pub name: String,
    pub host_suffix: Option<String>,
    pub path_prefix: Option<String>,
    pub mime_prefix: Option<String>,
    pub target: RouteTarget,
}

impl RouteRule {
    pub fn matches(&self, signals: &PageSignals) -> bool {
        if let Some(host_suffix) = &self.host_suffix {
            let Some(host) = signals.url.host_str() else {
                return false;
            };

            if !host
                .to_ascii_lowercase()
                .ends_with(&host_suffix.to_ascii_lowercase())
            {
                return false;
            }
        }

        if let Some(path_prefix) = &self.path_prefix {
            if !signals.url.path().starts_with(path_prefix) {
                return false;
            }
        }

        if let Some(mime_prefix) = &self.mime_prefix {
            let Some(mime) = &signals.mime_type else {
                return false;
            };

            if !mime
                .to_ascii_lowercase()
                .starts_with(&mime_prefix.to_ascii_lowercase())
            {
                return false;
            }
        }

        true
    }
}

#[derive(Clone, Debug)]
pub struct RouterConfig {
    pub semantic_threshold: f32,
    pub servo: EngineCapabilities,
    pub chromium: EngineCapabilities,
    pub rules: Vec<RouteRule>,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            semantic_threshold: 0.82,
            servo: EngineCapabilities::servo_conservative(),
            chromium: EngineCapabilities::chromium_compatibility(),
            rules: Vec::new(),
        }
    }
}

pub struct AdaptiveRouter {
    config: RouterConfig,
}

impl AdaptiveRouter {
    pub fn new(config: RouterConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &RouterConfig {
        &self.config
    }

    pub fn decide(&self, signals: &PageSignals) -> RouteDecision {
        // 1. Explicit runtime instruction.
        if let Some(target) = signals.force_route {
            return RouteDecision::new(target, "explicit route override");
        }

        // 2. Rule table.
        for rule in &self.config.rules {
            if rule.matches(signals) {
                return RouteDecision::new(
                    rule.target,
                    format!("matched route rule '{}'", rule.name),
                );
            }
        }

        // 3. Non-network protocols are not automatically interpreted
        // as semantic documents.
        match signals.url.scheme() {
            "http" | "https" => {}
            scheme => {
                return RouteDecision::new(
                    RouteTarget::Chromium,
                    format!(
                        "non-standard web scheme '{}' sent to compatibility runtime",
                        scheme
                    ),
                );
            }
        }

        // 4. Prefer Neroa semantic execution whenever browser runtime
        // semantics are unnecessary.
        if self.can_use_semantic(signals) {
            return RouteDecision::new(
                RouteTarget::Semantic,
                format!(
                    "semantic confidence {:.3} >= {:.3}",
                    signals.semantic_confidence, self.config.semantic_threshold
                ),
            )
            .push_reason("no live browser execution requirement detected");
        }

        // 5. Servo is the preferred live engine.
        if self.config.servo.supports(&signals.requirements) {
            return RouteDecision::new(
                RouteTarget::Servo,
                "live execution required and Servo capability profile satisfies page requirements",
            );
        }

        // 6. Chromium exists strictly as compatibility escalation.
        if self.config.chromium.supports(&signals.requirements) {
            return RouteDecision::new(
                RouteTarget::Chromium,
                "Servo capability profile insufficient; escalated to Chromium compatibility bridge",
            );
        }

        // 7. Chromium remains the last compatibility attempt even if
        // the declared capability profile cannot guarantee success.
        RouteDecision::new(
            RouteTarget::Chromium,
            "no engine capability profile fully satisfies requirements; Chromium selected as last compatibility attempt",
        )
    }

    fn can_use_semantic(&self, signals: &PageSignals) -> bool {
        if signals.semantic_confidence < self.config.semantic_threshold {
            return false;
        }

        if signals.interaction_required
            || signals.script_required
            || signals.media_playback
            || signals.requirements.needs_live_runtime()
        {
            return false;
        }

        match signals.mime_type.as_deref() {
            Some(mime) => {
                let mime = mime.to_ascii_lowercase();

                mime.starts_with("text/html")
                    || mime.starts_with("application/json")
                    || mime.starts_with("text/plain")
                    || mime.starts_with("application/xml")
                    || mime.starts_with("text/xml")
            }

            None => true,
        }
    }
}
