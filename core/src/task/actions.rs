//! Lo que se le hace a una tarea entera desde su página, más allá de sus
//! campos de un toque. Por ahora, dejar escrito dónde quedó.
//!
//! Vino de `HandoffSection` en `app/src/components/TaskDetailDrawer.tsx`.

use super::{refused, TaskDetail, TaskPatch, Tasks};
use crate::ApiError;

/// Lo más que mide un «dónde quedó», en letras: el tope del servidor
/// (`UpdateTaskRequest.Handoff`). Es un resumen para leer de un vistazo.
pub const MAX_HANDOFF: usize = 2000;

impl Tasks {
    fn detail_or_refuse(&self) -> Result<TaskDetail, ApiError> {
        self.state().detail.ok_or_else(|| refused("no-task-open", "No task is open"))
    }

    /// Deja escrito dónde quedó la tarea abierta; en blanco lo quita.
    ///
    /// Se reemplaza entero, y el servidor lo firma con quién y cuándo: de su
    /// respuesta sale la firma que se enseña, no de aquí. Si dice lo mismo
    /// que ya decía no se pide nada (la app de siempre sí lo manda, y eso le
    /// cambia la fecha a un texto que nadie tocó). Uno más largo de lo que el
    /// servidor guarda no se manda.
    pub async fn set_handoff(&self, text: &str) -> Result<(), ApiError> {
        let text = text.trim();
        if text.chars().count() > MAX_HANDOFF {
            return Err(refused("handoff-too-long", "A handoff can be 2000 characters at most"));
        }
        let has = self.detail_or_refuse()?.task.handoff.unwrap_or_default();
        if has.trim() == text {
            return Ok(());
        }
        self.update(TaskPatch { handoff: Some(text.to_string()), ..TaskPatch::default() }).await
    }
}
