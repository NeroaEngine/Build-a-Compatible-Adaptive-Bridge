#![cfg(all(feature = "headless-http", feature = "servo-runtime"))]

use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::sync::RwLock;
use url::Url;
use uuid::Uuid;

use crate::{
    agent::AgentSurface,
    engine::{EngineError, LiveWebEngine},
    servo_runtime::ServoEngineProxy,
    types::{ActivityState, StoragePartitionId, ViewConfig, ViewId, Viewport},
};

#[derive(Clone, Debug)]
pub struct HeadlessProfileSnapshot {
    pub profile_id: String,
    pub view_id: ViewId,
    pub url: Url,
    pub viewport: Viewport,
}

#[derive(Clone, Debug)]
struct HeadlessProfile {
    view_id: ViewId,
    url: Url,
    viewport: Viewport,
}

/// Keeps authenticated scraper browser views alive and keyed by profile_id.
///
/// A profile maps directly to Neroa's StoragePartitionId. Reusing the live
/// view preserves the browser's in-memory cookies/storage for subsequent
/// headless requests. Persistence across process restart remains a separate
/// governed state-export concern; this manager deliberately does not serialize
/// credentials or cookies to disk.
#[derive(Clone)]
pub struct HeadlessSessionManager {
    servo: ServoEngineProxy,
    profiles: Arc<RwLock<HashMap<String, HeadlessProfile>>>,
}

impl HeadlessSessionManager {
    #[must_use]
    pub fn new(servo: ServoEngineProxy) -> Self {
        Self {
            servo,
            profiles: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn open_or_reuse(
        &self,
        profile_id: &str,
        url: Url,
        viewport: Viewport,
    ) -> Result<HeadlessProfileSnapshot, EngineError> {
        validate_profile_id(profile_id)?;

        if let Some(existing) = self.profiles.read().await.get(profile_id).cloned() {
            if existing.viewport != viewport {
                self.servo.resize(existing.view_id, viewport.clone()).await?;
            }

            let surface = AgentSurface::new(self.servo.clone(), existing.view_id);
            if existing.url != url {
                let loaded = surface
                    .navigate_and_wait(url.clone(), Duration::from_secs(30))
                    .await?;
                if !loaded {
                    return Err(EngineError::Internal(format!(
                        "headless navigation timed out for profile {profile_id}"
                    )));
                }
            }

            let mut profiles = self.profiles.write().await;
            profiles.insert(
                profile_id.to_owned(),
                HeadlessProfile {
                    view_id: existing.view_id,
                    url: url.clone(),
                    viewport: viewport.clone(),
                },
            );

            return Ok(HeadlessProfileSnapshot {
                profile_id: profile_id.to_owned(),
                view_id: existing.view_id,
                url,
                viewport,
            });
        }

        let view_id = self
            .servo
            .create_view(ViewConfig {
                node_id: Uuid::new_v4(),
                initial_url: url.clone(),
                viewport: viewport.clone(),
                storage_partition: StoragePartitionId::new(profile_id),
            })
            .await?;

        self.servo
            .set_activity(view_id, ActivityState::Active)
            .await?;

        let surface = AgentSurface::new(self.servo.clone(), view_id);
        if !surface.wait_for_load(Duration::from_secs(30)).await? {
            let _ = self.servo.destroy_view(view_id).await;
            return Err(EngineError::Internal(format!(
                "initial headless load timed out for profile {profile_id}"
            )));
        }

        self.profiles.write().await.insert(
            profile_id.to_owned(),
            HeadlessProfile {
                view_id,
                url: url.clone(),
                viewport: viewport.clone(),
            },
        );

        Ok(HeadlessProfileSnapshot {
            profile_id: profile_id.to_owned(),
            view_id,
            url,
            viewport,
        })
    }

    pub async fn agent_surface(&self, profile_id: &str) -> Result<AgentSurface, EngineError> {
        let profiles = self.profiles.read().await;
        let profile = profiles.get(profile_id).ok_or_else(|| {
            EngineError::Internal(format!("headless profile not found: {profile_id}"))
        })?;
        Ok(AgentSurface::new(self.servo.clone(), profile.view_id))
    }

    pub async fn close(&self, profile_id: &str) -> Result<bool, EngineError> {
        let profile = self.profiles.write().await.remove(profile_id);
        if let Some(profile) = profile {
            self.servo.destroy_view(profile.view_id).await?;
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn list(&self) -> Vec<HeadlessProfileSnapshot> {
        self.profiles
            .read()
            .await
            .iter()
            .map(|(profile_id, profile)| HeadlessProfileSnapshot {
                profile_id: profile_id.clone(),
                view_id: profile.view_id,
                url: profile.url.clone(),
                viewport: profile.viewport.clone(),
            })
            .collect()
    }
}

fn validate_profile_id(profile_id: &str) -> Result<(), EngineError> {
    let valid = !profile_id.is_empty()
        && profile_id.len() <= 256
        && profile_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'));

    if valid {
        Ok(())
    } else {
        Err(EngineError::Internal("invalid headless profile_id".to_owned()))
    }
}
