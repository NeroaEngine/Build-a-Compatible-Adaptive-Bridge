use crate::types::RouteTarget;
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceRequest {
    pub request_id: Uuid,
    pub node_id: Uuid,
    pub url: Url,
    pub method: String,
    pub resource_type: ResourceType,
    pub response_mime: Option<String>,
}

impl ResourceRequest {
    pub fn new(node_id: Uuid, url: Url, resource_type: ResourceType) -> Self {
        Self {
            request_id: Uuid::new_v4(),
            node_id,
            url,
            method: "GET".into(),
            resource_type,
            response_mime: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResourceType {
    Document,
    Script,
    Json,
    Stylesheet,
    Image,
    Media,
    Font,
    WebSocket,
    Fetch,
    Xhr,
    Other,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NetworkAction {
    PassThrough,

    /// Allow request to continue while mirroring useful body/data to
    /// the semantic ingestion system.
    MirrorToAsg,

    /// Capture structured response such as JSON without changing the
    /// browser renderer.
    CaptureStructured,

    /// Block a request at the bridge.
    Block {
        reason: String,
    },

    /// Force the owning node onto another renderer.
    ForceRoute {
        target: RouteTarget,
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkRouteBlock {
    pub name: String,

    pub host_suffix: Option<String>,
    pub path_prefix: Option<String>,
    pub method: Option<String>,
    pub mime_prefix: Option<String>,
    pub resource_type: Option<ResourceType>,

    pub action: NetworkAction,
}

impl NetworkRouteBlock {
    pub fn matches(&self, request: &ResourceRequest) -> bool {
        if let Some(host_suffix) = &self.host_suffix {
            let Some(host) = request.url.host_str() else {
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
            if !request.url.path().starts_with(path_prefix) {
                return false;
            }
        }

        if let Some(method) = &self.method {
            if !request.method.eq_ignore_ascii_case(method) {
                return false;
            }
        }

        if let Some(resource_type) = self.resource_type {
            if resource_type != request.resource_type {
                return false;
            }
        }

        if let Some(mime_prefix) = &self.mime_prefix {
            let Some(mime) = &request.response_mime else {
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceDecision {
    pub request_id: Uuid,

    /// Browser request continues.
    pub continue_network: bool,

    /// Copy semantic representation/data into ASG ingestion.
    pub mirror_to_asg: bool,

    /// Capture structured response body when available.
    pub capture_structured: bool,

    /// If populated, renderer must be reconsidered.
    pub force_route: Option<RouteTarget>,

    pub reason: String,
}

impl ResourceDecision {
    pub fn pass(request_id: Uuid) -> Self {
        Self {
            request_id,
            continue_network: true,
            mirror_to_asg: false,
            capture_structured: false,
            force_route: None,
            reason: "pass through".into(),
        }
    }
}

#[derive(Default)]
pub struct NetworkRouter {
    blocks: Vec<NetworkRouteBlock>,
}

impl NetworkRouter {
    pub fn new(blocks: Vec<NetworkRouteBlock>) -> Self {
        Self { blocks }
    }

    pub fn add_block(&mut self, block: NetworkRouteBlock) {
        self.blocks.push(block);
    }

    pub fn evaluate(&self, request: &ResourceRequest) -> ResourceDecision {
        for block in &self.blocks {
            if block.matches(request) {
                return self.apply_action(request, &block.name, &block.action);
            }
        }

        self.default_decision(request)
    }

    fn apply_action(
        &self,
        request: &ResourceRequest,
        block_name: &str,
        action: &NetworkAction,
    ) -> ResourceDecision {
        match action {
            NetworkAction::PassThrough => ResourceDecision {
                request_id: request.request_id,
                continue_network: true,
                mirror_to_asg: false,
                capture_structured: false,
                force_route: None,
                reason: format!("network route block '{}': pass", block_name),
            },

            NetworkAction::MirrorToAsg => ResourceDecision {
                request_id: request.request_id,
                continue_network: true,
                mirror_to_asg: true,
                capture_structured: false,
                force_route: None,
                reason: format!("network route block '{}': mirror to ASG", block_name),
            },

            NetworkAction::CaptureStructured => ResourceDecision {
                request_id: request.request_id,
                continue_network: true,
                mirror_to_asg: true,
                capture_structured: true,
                force_route: None,
                reason: format!(
                    "network route block '{}': capture structured response",
                    block_name
                ),
            },

            NetworkAction::Block { reason } => ResourceDecision {
                request_id: request.request_id,
                continue_network: false,
                mirror_to_asg: false,
                capture_structured: false,
                force_route: None,
                reason: format!("network route block '{}': blocked: {}", block_name, reason),
            },

            NetworkAction::ForceRoute { target, reason } => ResourceDecision {
                request_id: request.request_id,
                continue_network: true,
                mirror_to_asg: false,
                capture_structured: false,
                force_route: Some(*target),
                reason: format!(
                    "network route block '{}': force {:?}: {}",
                    block_name, target, reason
                ),
            },
        }
    }

    fn default_decision(&self, request: &ResourceRequest) -> ResourceDecision {
        match request.resource_type {
            // JSON/XHR/fetch are extremely valuable to Neroa's
            // semantic ingestion system.
            ResourceType::Json | ResourceType::Xhr | ResourceType::Fetch => ResourceDecision {
                request_id: request.request_id,
                continue_network: true,
                mirror_to_asg: true,
                capture_structured: true,
                force_route: None,
                reason: "structured network response eligible for semantic capture".into(),
            },

            // Documents can feed both the live renderer and ASG.
            ResourceType::Document => ResourceDecision {
                request_id: request.request_id,
                continue_network: true,
                mirror_to_asg: true,
                capture_structured: false,
                force_route: None,
                reason: "document continues to renderer while semantic ingestion observes it"
                    .into(),
            },

            _ => ResourceDecision::pass(request.request_id),
        }
    }
}
