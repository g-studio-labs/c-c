//! Entrar a trabajar: lo que hay que traer cuando alguien está dentro, y al
//! cambiar de organización.
//!
//! Estaba repartido por los `useEffect` de la app de Tauri (`AppLayout.tsx`,
//! `useNavData.ts`, cada store). Aquí es un solo sitio, para que una app
//! nueva no tenga que redescubrir el orden.

use crate::Core;

impl Core {
    /// Al entrar, o al arrancar con sesión: abre el stream de eventos, trae
    /// las organizaciones y lo de la que se mira.
    pub async fn open_workspace(&self) {
        // El stream primero: lo que cambie mientras llega lo demás ya se oye.
        self.stream().start();
        self.orgs().load().await;
        self.load_current_org().await;
    }

    /// Cambia de organización y trae lo suyo. Una que no es de esta persona
    /// no se acepta, y entonces no se pide nada.
    pub async fn switch_org(&self, org_id: &str) -> bool {
        // Con documentación a medio escribir no se cambia: se pregunta antes
        // (`doc_edit`), y cambiar es lo que se hace si contestan que sí.
        if self.docs().held_back(crate::doc_edit::DocLeave::Org { id: org_id.to_string() }) {
            return false;
        }
        if !self.orgs().set_current(org_id) {
            return false;
        }
        // El documento y la tarea abiertos son de la organización que se deja.
        self.docs().close_now();
        self.tasks().close();
        self.load_current_org().await;
        true
    }

    /// Cambia de lente («asignado a mí», «siguiendo»…) y trae sus tareas si
    /// lo cargado no responde ya a esa pregunta: cada lente es otra pregunta
    /// al servidor, no un filtro sobre lo descargado.
    pub async fn show_lens(&self, lens: crate::mywork::WorkLens) {
        self.my_work().set_lens(lens);
        self.my_work().ensure_loaded(self.orgs().state().current_org_id).await;
    }

    /// Enseña también lo terminado, o sólo lo abierto. Lo terminado no se
    /// pide hasta que alguien lo quiere ver.
    pub async fn show_closed(&self, on: bool) {
        self.my_work().set_include_closed(on);
        self.my_work().ensure_loaded(self.orgs().state().current_org_id).await;
    }

    /// El árbol, sus marcas de documentación, «Mi trabajo» y los contadores
    /// de la organización que se mira, a la vez.
    async fn load_current_org(&self) {
        let org_id = self.orgs().state().current_org_id;
        // Los contadores en su propia tarea, y también sin organización: a
        // quien todavía no es de ninguna hay que enseñarle sus invitaciones.
        let counters = {
            let (core, org_id) = (self.clone(), org_id.clone());
            tokio::spawn(async move { core.counters().load(org_id).await })
        };
        let Some(org_id) = org_id else {
            let _ = counters.await;
            return;
        };
        // «Mi trabajo» en su propia tarea, para que no espere al árbol.
        let my_work = {
            let (core, org_id) = (self.clone(), org_id.clone());
            tokio::spawn(async move { core.my_work().load(Some(org_id)).await })
        };
        // Las marcas después del árbol: son de él, y `load` las vacía al
        // cambiar de organización.
        self.tree().load(&org_id).await;
        self.tree().load_doc_marks(&org_id).await;
        let _ = my_work.await;
        let _ = counters.await;
    }
}
