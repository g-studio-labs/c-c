package i18n

// El catálogo del servidor.
//
// Deliberadamente corto: aquí sólo entra lo que **el servidor** escribe para
// una persona. Todo lo que se pinta en una pantalla vive en los catálogos del
// cliente, que es donde se puede cambiar de idioma sin desplegar nada.
//
// Las claves de error son **la etiqueta de código que ya viaja** en cada
// `SendErrorResponse` —`inbox-other-org`, `rate-limited`—, no una clave nueva.
// Esa etiqueta ya era el contrato estable y la frase ya estaba declarada como
// decorativa; lo único que cambia es quién escribe la frase.
var catalogo = map[string]map[Locale]string{
	// ── Avisos de la bandeja ────────────────────────────────────────────────
	// Se escriben una vez, en el idioma de quien los va a leer.
	"notify.dm.wrote": {
		EN: "{{who}} wrote to you",
		ES: "{{who}} te escribió",
	},
	"notify.dm.new": {
		EN: "New direct message",
		ES: "Mensaje directo nuevo",
	},
	// Corregir un mensaje que ya avisó (command-and-control-547). Con agente,
	// la firma de la 522: «el agente de Ana», no Ana.
	"notify.message.edited": {
		EN: "{{who}} edited their message",
		ES: "{{who}} editó su mensaje",
	},
	"notify.message.edited.agent": {
		EN: "{{who}}'s agent edited their message",
		ES: "El agente de {{who}} editó su mensaje",
	},
	"notify.message.edited.in": {
		EN: "{{who}} edited their message in {{where}}",
		ES: "{{who}} editó su mensaje en {{where}}",
	},
	"notify.message.edited.agent.in": {
		EN: "{{who}}'s agent edited their message in {{where}}",
		ES: "El agente de {{who}} editó su mensaje en {{where}}",
	},

	// Un agente —o alguien— cambió un documento y pide que se lea.
	"notify.doc.review": {
		EN: "Review requested · {{doc}}",
		ES: "Revisión pedida · {{doc}}",
	},
	// Un agente dejó un pedido para esta persona (domain/request.go).
	"notify.request.new": {
		EN: "Waiting on you · {{task}}",
		ES: "Te espera · {{task}}",
	},
	"notify.report.new": {
		EN: "New report · {{folio}}",
		ES: "Reporte nuevo · {{folio}}",
	},
	"notify.reply.new": {
		EN: "New reply",
		ES: "Respuesta nueva",
	},
	"notify.reply.by": {
		EN: "{{who}} replied",
		ES: "{{who}} respondió",
	},
	"notify.reply.client": {
		EN: "The client replied",
		ES: "El cliente respondió",
	},
	"notify.item.moved": {
		EN: "Moved to {{status}}",
		ES: "Movida a {{status}}",
	},
	"notify.chat.mentioned": {
		EN: "Mentioned in {{where}}",
		ES: "Te nombraron en {{where}}",
	},
	"notify.chat.replied": {
		EN: "Replied to you in {{where}}",
		ES: "Te respondieron en {{where}}",
	},
	// Basecamp (command-and-control-516): lo que pasa allí llega aquí.
	"notify.basecamp.comment": {
		EN: "{{who}} commented in Basecamp",
		ES: "{{who}} comentó en Basecamp",
	},
	"notify.basecamp.message": {
		EN: "{{who}} posted a message in Basecamp",
		ES: "{{who}} publicó un mensaje en Basecamp",
	},
	"notify.basecamp.todo_done": {
		EN: "{{who}} completed a to-do in Basecamp",
		ES: "{{who}} completó un to-do en Basecamp",
	},
	"notify.basecamp.todo_created": {
		EN: "{{who}} created a to-do in Basecamp",
		ES: "{{who}} creó un to-do en Basecamp",
	},
	"notify.basecamp.todo_reopened": {
		EN: "{{who}} reopened a to-do in Basecamp",
		ES: "{{who}} reabrió un to-do en Basecamp",
	},
	// Un to-do de Basecamp que llega como tarea sin nadie de cac a cargo: se le
	// dice a quien enlazó el proyecto (command-and-control-557).
	"notify.basecamp.todo_unassigned": {
		EN: "{{who}} left a Basecamp to-do with nobody assigned",
		ES: "{{who}} dejó un to-do de Basecamp sin nadie asignado",
	},
	"notify.basecamp.todo_unmatched": {
		EN: "{{who}} assigned a Basecamp to-do to someone who is not linked to cac",
		ES: "{{who}} asignó un to-do de Basecamp a alguien que no está enlazado en cac",
	},
	"notify.basecamp.mention": {
		EN: "{{who}} mentioned you in Basecamp",
		ES: "{{who}} te nombró en Basecamp",
	},
	// Un comentario en Basecamp de quien tiene varios pedidos abiertos en la
	// tarea: no se adivina a cuál contesta.
	"notify.basecamp.request_ambiguous": {
		EN: "Your Basecamp comment did not answer your {{count}} open requests: answer them in cac",
		ES: "Tu comentario en Basecamp no contestó tus {{count}} pedidos abiertos: contéstalos en cac",
	},
	"notify.basecamp.auth_failed": {
		EN: "Basecamp stopped renewing: connect it again",
		ES: "Basecamp dejó de renovarse: vuelve a conectarlo",
	},
	"notify.item.assigned": {
		EN: "Assigned to you",
		ES: "Te la asignaron",
	},
	"notify.item.moved.open": {
		EN: "Reopened",
		ES: "Reabierta",
	},
	"notify.item.moved.in_progress": {
		EN: "Moved to In progress",
		ES: "Movida a En curso",
	},
	"notify.item.moved.done": {
		EN: "Marked as done",
		ES: "Marcada como hecha",
	},
	"notify.item.moved.closed": {
		EN: "Closed",
		ES: "Cerrada",
	},

	// La actividad de CI (R9): un run que terminó, un deploy que acabó.
	"notify.ci.run.success": {
		EN: "{{workflow}} passed on {{branch}}",
		ES: "{{workflow}} pasó en {{branch}}",
	},
	"notify.ci.run.failure": {
		EN: "{{workflow}} failed on {{branch}}",
		ES: "{{workflow}} falló en {{branch}}",
	},
	"notify.ci.run.cancelled": {
		EN: "{{workflow}} was cancelled on {{branch}}",
		ES: "{{workflow}} se canceló en {{branch}}",
	},
	"notify.ci.run": {
		EN: "{{workflow}} finished: {{conclusion}} · {{branch}}",
		ES: "{{workflow}} terminó: {{conclusion}} · {{branch}}",
	},
	"notify.deploy.succeeded": {
		EN: "{{service}} deployed · {{sha}}",
		ES: "{{service}} desplegado · {{sha}}",
	},
	"notify.deploy.failed": {
		EN: "{{service}} deploy failed · {{sha}}",
		ES: "Falló el deploy de {{service}} · {{sha}}",
	},
	"notify.deploy.expired": {
		EN: "{{service}} deploy expired: the agent stopped responding · {{sha}}",
		ES: "Caducó el deploy de {{service}}: el agente dejó de contestar · {{sha}}",
	},
	// Los backups de bases (#166): a los admins de la organización.
	"notify.backup.failed": {
		EN: "Backup of {{db}} failed · {{server}}",
		ES: "Falló el backup de {{db}} · {{server}}",
	},
	"notify.backup.stale": {
		EN: "{{db}} has had no backup for {{hours}} h · {{server}}",
		ES: "{{db}} lleva {{hours}} h sin backup · {{server}}",
	},
	"notify.backup.restoreFailed": {
		EN: "Restore test of {{db}} failed · {{server}}",
		ES: "Falló la prueba de restauración de {{db}} · {{server}}",
	},
	"notify.telemetry.silent": {
		EN: "{{device}} stopped sending",
		ES: "{{device}} dejó de mandar",
	},
	"notify.telemetry.back": {
		EN: "{{device}} is sending again",
		ES: "{{device}} volvió a mandar",
	},
	"notify.telemetry.unhealthy": {
		EN: "{{device}} needs attention",
		ES: "{{device}} necesita atención",
	},
	"notify.telemetry.healthy": {
		EN: "{{device}} is healthy again",
		ES: "{{device}} volvió a estar bien",
	},

	// ── Errores de la API ───────────────────────────────────────────────────
	"unauthorized": {
		EN: "Unauthorized",
		ES: "Sin autorización",
	},
	"invalid-request": {
		EN: "Invalid request",
		ES: "Petición inválida",
	},
	"not-found": {
		EN: "Not found",
		ES: "No se encontró",
	},
	"forbidden": {
		EN: "Forbidden",
		ES: "Sin permiso",
	},
	"rate-limited": {
		EN: "Too many requests",
		ES: "Demasiadas peticiones",
	},
	"inbox-other-org": {
		EN: "That list belongs to another organization",
		ES: "Esa lista es de otra organización",
	},

	// ── Correos ──────────────────────────────────────────────────────────────
	// Un correo se escribe una vez, como un aviso: la bienvenida en el idioma
	// del admin que da de alta (la persona todavía no eligió el suyo),
	// restablecer en el de quien lo pide.
	"email.welcome.subject": {
		EN: "{{inviter}} created your cac account in {{org}}",
		ES: "{{inviter}} te creó una cuenta de cac en {{org}}",
	},
	"email.welcome.greeting": {
		EN: "Hi {{name}},",
		ES: "Hola, {{name}}:",
	},
	"email.welcome.body": {
		EN: "{{inviter}} created an account for you in {{org}}. Choose your username and password to get started.",
		ES: "{{inviter}} te creó una cuenta en {{org}}. Elige tu usuario y tu contraseña para empezar.",
	},
	"email.welcome.button": {
		EN: "Finish signing up",
		ES: "Terminar el registro",
	},
	"email.welcome.expires": {
		EN: "The link is valid for 7 days and works once.",
		ES: "El enlace vale 7 días y funciona una sola vez.",
	},
	"email.invited.subject": {
		EN: "{{inviter}} invited you to {{org}} on cac",
		ES: "{{inviter}} te invitó a {{org}} en cac",
	},
	"email.invited.body": {
		EN: "{{inviter}} invited you to join {{org}}. Open cac to accept or decline.",
		ES: "{{inviter}} te invitó a unirte a {{org}}. Abre cac para aceptar o rechazar.",
	},
	"email.invited.button": {
		EN: "Open cac",
		ES: "Abrir cac",
	},
	"email.reset.subject": {
		EN: "Reset your cac password",
		ES: "Restablece tu contraseña de cac",
	},
	"email.reset.body": {
		EN: "Someone asked to reset the password of {{username}}. If it was you, choose a new one.",
		ES: "Alguien pidió restablecer la contraseña de {{username}}. Si fuiste tú, elige una nueva.",
	},
	"email.reset.button": {
		EN: "Choose a new password",
		ES: "Elegir una contraseña nueva",
	},
	"email.reset.expires": {
		EN: "The link is valid for 1 hour and works once. If it wasn't you, ignore this email: your password stays the same.",
		ES: "El enlace vale 1 hora y funciona una sola vez. Si no fuiste tú, ignora este correo: tu contraseña no cambia.",
	},
	"email.link.fallback": {
		EN: "If the button doesn't work, paste this link into your browser:",
		ES: "Si el botón no funciona, pega este enlace en tu navegador:",
	},
	"email.footer": {
		EN: "You're getting this email because of your account on cac.",
		ES: "Te llega este correo por tu cuenta de cac.",
	},

	// ── El muro ──────────────────────────────────────────────────────────────
	"notify.post.feedback": {
		EN: "New feedback from {{org}}",
		ES: "Feedback nuevo de {{org}}",
	},
	"notify.post.draft": {
		EN: "Draft announcement ready to review",
		ES: "Borrador de comunicado listo para revisar",
	},
	"notify.post.comment": {
		EN: "{{who}} commented on a post",
		ES: "{{who}} comentó una publicación",
	},
}
