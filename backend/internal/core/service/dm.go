package service

import (
	"time"

	"github.com/google/uuid"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/events"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// Direct messages: two people, one organization.
//
// The authorship rule and the shape of every operation match the space
// channels, because writing is writing. What differs is who may see it, and
// that difference is carried in two places on purpose — the tables (which no
// channel query touches) and the event's address (which no third console
// receives).
type DMService struct {
	repo     *repository.DMRepository
	hub      *events.Hub
	notifier Notifier
}

func NewDMService(repo *repository.DMRepository, hub *events.Hub) *DMService {
	return &DMService{repo: repo, hub: hub}
}

// WithNotifier records what it publishes, so a message you missed is still
// there tomorrow. Optional on purpose: without one this behaves exactly as
// before, which is what keeps every existing test honest.
func (s *DMService) WithNotifier(n Notifier) *DMService {
	s.notifier = n
	return s
}

// publish tells one person that their conversation moved.
//
// Event.UserID, never a bare OrgID: the org-wide path would put a private
// message on every console in the organization and on every superadmin's
// stream. And the hub only, never emitItemEvent — that one dispatches the
// tenant's webhook, which is about as far outside this conversation as it gets.
//
// The author is not told about their own message; the actor rides along anyway
// so the receiving console can apply the same echo rule it uses everywhere.
//
// `edited` es una corrección o un retiro: el evento sale igual —la
// conversación cambia en pantalla— y marcado, pero sin fila en la campana:
// no es que te hayan escrito otra vez (command-and-control-418).
func (s *DMService) publish(toUserID, orgID, conversationID, messageID, actorID string, edited bool) {
	if s.hub == nil || toUserID == "" || toUserID == actorID {
		return
	}
	// El nombre viaja en el evento, y no se deja que la consola lo busque.
	//
	// Lo sacaba de su lista de conversaciones, que sólo está cargada si has
	// abierto la sección de directos; antes de eso el aviso del sistema decía
	// «Direct message» y no se distinguía de ningún otro. Mismo fallo que tenía
	// el chat con el nombre del canal, y misma solución.
	//
	// El **texto** no viaja, a diferencia del chat: ver el comentario de abajo.
	de := s.repo.NombreDe(actorID)
	s.hub.Publish(events.Event{
		Type:   "dm:message",
		UserID: toUserID,
		Data: map[string]any{
			"conversationId": conversationID, "messageId": messageID, "actorId": actorID,
			"authorName": de, "edited": edited,
		},
	})
	if s.notifier != nil && !edited {
		// El nombre de quien escribe en el título; el mensaje **no**.
		//
		// Sin el nombre, tres conversaciones abiertas daban tres filas
		// idénticas —«New direct message»— y había que entrar en las tres para
		// saber cuál era. Quién te escribe ya se ve en la lista de
		// conversaciones, así que decirlo aquí no destapa nada.
		//
		// El cuerpo sigue fuera, y a propósito: una fila de la bandeja la lee
		// alguien que puede tener la pantalla compartida, y lo que distingue a
		// una conversación privada es justo que su contenido se queda dentro.
		// En un canal el texto sí viaja, porque ahí ya lo puede leer toda la
		// organización.
		//
		// ViaApp fijo, por lo mismo que en chat.go: ninguna herramienta del MCP
		// escribe directos. Si alguna llega, este servicio necesita el contexto.
		// La frase no se arma aquí sino en el notificador, que es el único que
		// sabe en qué idioma lee `toUserID`. Antes esta línea decía «te
		// escribió» en castellano dentro de una aplicación en inglés, que era
		// el mismo fallo visto desde el otro lado.
		clave, args := "notify.dm.new", map[string]string(nil)
		if de != "" {
			clave, args = "notify.dm.wrote", map[string]string{"who": de}
		}
		s.notifier.Notify(domain.Aviso{
			UserID: toUserID, OrgID: orgID, Kind: "dm:message",
			TitleKey: clave, TitleArgs: args,
			Body: "", Link: "/dm?c=" + conversationID, Via: domain.ViaApp,
			// El rótulo es el nombre a secas, sin el «te escribió» del título:
			// plegados, tres directos de Ana son «Ana», no «Ana te escribió».
			Group: domain.DMGroup(conversationID), Label: de,
		})
	}
}

// OpenWith is how a conversation starts: there is no "create", only naming the
// person. Both directions resolve to the same row.
func (s *DMService) OpenWith(orgID, userID, otherID string) (*domain.DMConversation, error) {
	return s.repo.OpenWith(orgID, userID, otherID)
}

// mine loads a conversation and refuses it to anyone who is not in it.
//
// Not-found rather than forbidden, like the spaces: telling somebody that a
// conversation exists but isn't theirs is itself a fact about who talks to whom.
func (s *DMService) mine(conversationID, userID string) (*domain.DMConversation, error) {
	c, err := s.repo.FindConversation(conversationID)
	if err != nil {
		return nil, err
	}
	if !c.Includes(userID) {
		return nil, repository.ErrConversationNotFound
	}
	return c, nil
}

func (s *DMService) List(conversationID, userID string, before time.Time, limit int) ([]domain.DMMessageResponse, error) {
	if _, err := s.mine(conversationID, userID); err != nil {
		return nil, err
	}
	msgs, err := s.repo.List(conversationID, before, limit)
	if err != nil {
		return nil, err
	}
	ids := make([]string, len(msgs))
	for i, m := range msgs {
		ids[i] = m.ID
	}
	por, err := s.repo.Reactions().Summaries(domain.ReactionDM, ids, userID)
	if err != nil {
		return nil, err
	}
	for i := range msgs {
		msgs[i].Reactions = por[msgs[i].ID]
		if msgs[i].Reactions == nil {
			msgs[i].Reactions = []domain.ReactionSummary{}
		}
	}
	return msgs, nil
}

// React pone o quita un emoji en un mensaje de la conversación. Cualquiera de
// los dos puede reaccionar a cualquier mensaje de los dos; lo que no puede es
// reaccionar a uno de **otra** conversación pasando su id por la suya.
func (s *DMService) React(conversationID, messageID, userID, emoji string, on bool) error {
	if !domain.ValidReaction(emoji) {
		return ErrBadReaction
	}
	c, err := s.mine(conversationID, userID)
	if err != nil {
		return err
	}
	m, err := s.repo.Find(messageID)
	if err != nil {
		return err
	}
	if m.ConversationID != c.ID {
		return repository.ErrDMMessageNotFound
	}
	if err := s.repo.Reactions().Set(domain.ReactionDM, m.ID, userID, emoji, on); err != nil {
		return err
	}
	// A las dos personas y a nadie más, como los mensajes (ver publish). A
	// quien reaccionó también: sus otras pantallas tienen que enterarse. Sin
	// aviso en la bandeja: una reacción no es un mensaje.
	if s.hub != nil {
		for _, u := range []string{c.UserLoID, c.UserHiID} {
			s.hub.Publish(events.Event{
				Type:   "dm:reaction",
				UserID: u,
				Data: map[string]string{
					"conversationId": c.ID, "messageId": m.ID, "actorId": userID,
				},
			})
		}
	}
	return nil
}

func (s *DMService) Post(conversationID, userID, body string) (*domain.DMMessage, error) {
	return s.Reply(conversationID, userID, body, "")
}

// Reply escribe una línea que contesta a otra de la misma conversación. Con
// replyToID vacío es Post. La misma guarda que en un canal (ver
// ChatService.Reply): lo citado tiene que ser de esta conversación y seguir en
// pie, porque la cita se lee con un join que no lo comprueba.
//
// No avisa aparte: en un directo el otro ya recibe el aviso del mensaje.
func (s *DMService) Reply(conversationID, userID, body, replyToID string) (*domain.DMMessage, error) {
	return s.ReplyVia(domain.ViaApp, conversationID, userID, body, replyToID)
}

// ReplyVia es Reply diciendo por dónde entró (ver domain/via.go).
func (s *DMService) ReplyVia(via, conversationID, userID, body, replyToID string) (*domain.DMMessage, error) {
	c, err := s.mine(conversationID, userID)
	if err != nil {
		return nil, err
	}
	m := &domain.DMMessage{
		ConversationID: c.ID, OrgID: c.OrgID, AuthorUserID: userID, Body: body,
		Via: domain.NormalizeVia(via),
	}
	if replyToID != "" {
		target, err := s.repo.Find(replyToID)
		if err != nil || target.ConversationID != c.ID {
			return nil, ErrReplyElsewhere
		}
		m.ReplyToID = &target.ID
	}
	m.ID = uuid.NewString()
	if err := s.repo.Create(m); err != nil {
		return nil, err
	}
	s.publish(c.Other(userID), c.OrgID, c.ID, m.ID, userID, false)
	return m, nil
}

func (s *DMService) Edit(conversationID, messageID, userID string, superadmin bool, body string) error {
	return s.EditAs(domain.MessageChanger{UserID: userID, Superadmin: superadmin}, conversationID, messageID, body)
}

// own: el mensaje de esta conversación que quien cambia puede tocar.
func (s *DMService) own(by domain.MessageChanger, conversationID, messageID string) (*domain.DMConversation, *domain.DMMessage, error) {
	c, err := s.mine(conversationID, by.UserID)
	if err != nil {
		return nil, nil, err
	}
	m, err := s.repo.Find(messageID)
	if err != nil {
		return nil, nil, err
	}
	// Belonging to the conversation is not enough: rewriting what the other
	// person said is not a thing either of them may do.
	if m.ConversationID != c.ID {
		return nil, nil, repository.ErrDMMessageNotFound
	}
	if m.AuthorUserID != by.UserID && !by.MayChangeOthers() {
		return nil, nil, ErrNotTheAuthor
	}
	return c, m, nil
}

// EditAs corrige diciendo quién y por dónde (command-and-control-547).
func (s *DMService) EditAs(by domain.MessageChanger, conversationID, messageID, body string) error {
	c, m, err := s.own(by, conversationID, messageID)
	if err != nil {
		return err
	}
	if err := s.repo.Rewrite(messageID, body, by.WrittenVia()); err != nil {
		return err
	}
	s.publish(c.Other(by.UserID), c.OrgID, c.ID, m.ID, by.UserID, true)
	s.announceEdit(c, m, by)
	return nil
}

// announceEdit: «Ana editó su mensaje», al otro de la conversación
// (command-and-control-547). **Sin el texto**, por lo mismo que el aviso de
// un directo nuevo (ver publish): una fila de la campana la puede leer quien
// mire la pantalla. Sólo si corrige el autor: lo que limpia un superadmin no
// es «su mensaje».
func (s *DMService) announceEdit(c *domain.DMConversation, m *domain.DMMessage, by domain.MessageChanger) {
	to := c.Other(by.UserID)
	if s.notifier == nil || m.AuthorUserID != by.UserID || to == "" || to == by.UserID {
		return
	}
	who := s.repo.NombreDe(by.UserID)
	key := "notify.message.edited"
	if by.AsAgent() {
		key = "notify.message.edited.agent"
	}
	s.notifier.Notify(domain.Aviso{
		UserID: to, OrgID: c.OrgID, Kind: domain.MessageEditedDM,
		TitleKey: key, TitleArgs: map[string]string{"who": who},
		Body: "", Link: "/dm?c=" + c.ID, Via: by.WrittenVia(),
		Group: domain.DMGroup(c.ID), Label: who,
	})
}

func (s *DMService) Withdraw(conversationID, messageID, userID string, superadmin bool) error {
	return s.WithdrawAs(domain.MessageChanger{UserID: userID, Superadmin: superadmin}, conversationID, messageID)
}

// WithdrawAs retira diciendo quién. Un token sólo lo que escribió un agente;
// sin aviso en la campana (command-and-control-547).
func (s *DMService) WithdrawAs(by domain.MessageChanger, conversationID, messageID string) error {
	c, m, err := s.own(by, conversationID, messageID)
	if err != nil {
		return err
	}
	if !by.MayWithdraw(m.CreatedVia) {
		return ErrTokenWithdrawsOnlyAgentWords
	}
	if err := s.repo.Withdraw(messageID); err != nil {
		return err
	}
	s.publish(c.Other(by.UserID), c.OrgID, c.ID, m.ID, by.UserID, true)
	return nil
}

func (s *DMService) MarkRead(conversationID, userID string) error {
	c, err := s.mine(conversationID, userID)
	if err != nil {
		return err
	}
	if err := s.repo.MarkRead(conversationID, userID); err != nil {
		return err
	}
	// Al otro, para que sus palomitas pasen a leído sin recargar. Sólo a él:
	// hasta dónde lee alguien una conversación privada no es de nadie más.
	if s.hub != nil {
		s.hub.Publish(events.Event{
			Type: "dm:read", UserID: c.Other(userID),
			Data: domain.DMReadEvent{ConversationID: c.ID, UserID: userID, At: time.Now().UTC()},
		})
	}
	return nil
}

func (s *DMService) Conversations(userID string, orgIDs []string) ([]domain.DMSummary, error) {
	return s.repo.Conversations(userID, orgIDs)
}

// Member: nil si esa persona es de la conversación; si no, como si no
// existiera.
func (s *DMService) Member(conversationID, userID string) error {
	_, err := s.mine(conversationID, userID)
	return err
}

// AddAttachment guarda un adjunto de una conversación, si quien lo sube es de
// ella.
func (s *DMService) AddAttachment(conversationID, userID string, a *domain.DMAttachment) error {
	if _, err := s.mine(conversationID, userID); err != nil {
		return err
	}
	a.ConversationID = conversationID
	a.UploadedBy = userID
	return s.repo.AddAttachment(a)
}

// Attachment: un adjunto, **sólo** para las dos personas de su conversación.
// A cualquier otra —también de la misma org, también admin— le contesta lo
// mismo que si no existiera: un directo es privado, y saber que hay un fichero
// ahí ya es saber algo de quién habla con quién.
func (s *DMService) Attachment(conversationID, attachmentID, userID string) (*domain.DMAttachment, error) {
	if _, err := s.mine(conversationID, userID); err != nil {
		return nil, err
	}
	a, err := s.repo.FindAttachment(attachmentID)
	if err != nil {
		return nil, err
	}
	if a.ConversationID != conversationID {
		return nil, repository.ErrDMAttachmentNotFound
	}
	return a, nil
}
