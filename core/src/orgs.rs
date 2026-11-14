//! Las organizaciones de quien está dentro, y cuál está mirando.
//!
//! Vino de `app/src/store/orgs.store.ts`. Casi todo lo demás se pide «para
//! una organización» (`?orgId=`), y la zona de la organización decide cuándo
//! es «hoy» para un vencimiento, así que esto va antes que cualquier pantalla.

use serde::{Deserialize, Serialize};

use crate::Core;

const STORAGE_KEY: &str = "orgs";

/// Una organización, vista por quien pregunta. Espejo de
/// `domain.OrganizationResponse`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct Organization {
    pub id: String,
    pub name: String,
    pub slug: String,
    /// El papel de quien pregunta en esta organización: `admin`, `member` o
    /// `viewer`.
    pub role: String,
    #[serde(default)]
    pub member_count: i64,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_invite_role: Option<String>,
    #[serde(default)]
    pub clients_see_only_their_space: bool,
    #[serde(default)]
    pub guests_can_use_dev_tools: bool,
    #[serde(default)]
    pub done_needs_subtasks_done: bool,
    /// La zona horaria de la organización (`America/Mexico_City`). Vacía es
    /// «no la han puesto»: entonces «hoy» es el de quien mira.
    #[serde(default)]
    pub timezone: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct OrgsState {
    pub orgs: Vec<Organization>,
    pub current_org_id: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
}

impl OrgsState {
    pub fn current(&self) -> Option<&Organization> {
        let id = self.current_org_id.as_deref()?;
        self.orgs.iter().find(|org| org.id == id)
    }

    /// La zona de la organización que se mira, si tiene una puesta.
    pub fn current_zone(&self) -> Option<&str> {
        self.current().map(|org| org.timezone.as_str()).filter(|zone| !zone.is_empty())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Remembered {
    current_org_id: Option<String>,
}

/// Las organizaciones de este core.
pub struct Orgs {
    core: Core,
}

impl Core {
    pub fn orgs(&self) -> Orgs {
        Orgs { core: self.clone() }
    }
}

impl Orgs {
    pub fn state(&self) -> OrgsState {
        self.core.inner.orgs.lock().expect("orgs lock").clone()
    }

    fn update(&self, change: impl FnOnce(&mut OrgsState)) {
        let state = {
            let mut state = self.core.inner.orgs.lock().expect("orgs lock");
            change(&mut state);
            state.clone()
        };
        self.core.inner.listener.orgs_changed(state);
    }

    /// La que se miraba la última vez, para arrancar en ella.
    pub(crate) fn restore(&self) {
        let Some(raw) = self.core.inner.storage.get(STORAGE_KEY) else { return };
        let Ok(saved) = serde_json::from_str::<Remembered>(&raw) else { return };
        self.core.inner.orgs.lock().expect("orgs lock").current_org_id = saved.current_org_id;
    }

    fn remember(&self) {
        let saved = Remembered { current_org_id: self.state().current_org_id };
        if let Ok(raw) = serde_json::to_string(&saved) {
            let _ = self.core.inner.storage.set(STORAGE_KEY, &raw);
        }
    }

    /// Pide las organizaciones y decide cuál se mira.
    ///
    /// La que se miraba, si sigue siendo de esta persona; si no —la sacaron,
    /// o es la primera vez— la primera de la lista. Quedarse con una que ya
    /// no es suya dejaría todas las pantallas pidiendo datos de una
    /// organización que el servidor le niega.
    pub async fn load(&self) {
        self.update(|state| {
            state.loading = true;
            state.error = None;
        });
        match self.core.get::<Vec<Organization>>("/api/v1/organizations/").await {
            Ok(orgs) => {
                self.update(|state| {
                    let still_mine = state
                        .current_org_id
                        .as_ref()
                        .is_some_and(|id| orgs.iter().any(|org| &org.id == id));
                    if !still_mine {
                        state.current_org_id = orgs.first().map(|org| org.id.clone());
                    }
                    state.orgs = orgs;
                    state.loading = false;
                });
                self.remember();
            }
            // La lista que hubiera se queda: un fallo de red no es «no
            // perteneces a ninguna».
            Err(error) => self.update(|state| {
                state.error = Some(error.to_string());
                state.loading = false;
            }),
        }
    }

    /// Cambia de organización. Una que no es de esta persona no se acepta.
    pub fn set_current(&self, org_id: &str) -> bool {
        if !self.state().orgs.iter().any(|org| org.id == org_id) {
            return false;
        }
        self.update(|state| state.current_org_id = Some(org_id.to_string()));
        self.remember();
        true
    }
}
