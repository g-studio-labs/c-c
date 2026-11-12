package repository

import (
	"time"

	"github.com/google/uuid"
	"gorm.io/gorm"
	"gorm.io/gorm/clause"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

// AgentUsageRepository: las llamadas de los agentes y lo que enseñaron los
// hooks (ver domain.AgentCall).
type AgentUsageRepository struct{ db *gorm.DB }

func NewAgentUsageRepository(db *gorm.DB) *AgentUsageRepository { return &AgentUsageRepository{db: db} }

// RecordCall apunta una llamada. La misma `callID` otra vez —la siguiente
// petición de la misma herramienta— no suma.
func (r *AgentUsageRepository) RecordCall(userID, tool, callID, target string) error {
	tool, target = domain.NormalizeAgentTool(tool), domain.NormalizeAgentTarget(target)
	if userID == "" || tool == "" || callID == "" || len(callID) > 64 {
		return nil
	}
	return r.db.Clauses(clause.OnConflict{Columns: []clause.Column{{Name: "call_id"}}, DoNothing: true}).
		Create(&domain.AgentCall{ID: uuid.NewString(), CallID: callID, UserID: userID, Tool: tool, Target: target}).Error
}

// RecordShown apunta lo que un hook enseñó, una fila por cosa, y poda lo
// viejo (aquí y no en cada llamada: un hook habla mucho menos).
func (r *AgentUsageRepository) RecordShown(userID string, req domain.AgentShownRequest) error {
	hook := domain.NormalizeAgentTool(req.Hook)
	if userID == "" || hook == "" {
		return nil
	}
	rows := []domain.AgentCall{}
	for i, t := range req.Targets {
		if i == domain.AgentShownMax {
			break
		}
		if t = domain.NormalizeAgentTarget(t); t != "" {
			rows = append(rows, domain.AgentCall{ID: uuid.NewString(), CallID: uuid.NewString(), UserID: userID,
				Tool: domain.AgentHookPrefix + hook, Target: t})
		}
	}
	if len(rows) > 0 {
		if err := r.db.Create(&rows).Error; err != nil {
			return err
		}
	}
	return r.db.Where("created_at < ?", time.Now().Add(-domain.AgentCallRetention)).Delete(&domain.AgentCall{}).Error
}

// Usage: desde `since`, por herramienta y por hook. Con `userID`, sólo lo
// suyo; vacío, todo el mundo.
func (r *AgentUsageRepository) Usage(userID string, since time.Time) (domain.AgentUsage, error) {
	out := domain.AgentUsage{Since: since, Tools: []domain.AgentToolUsage{}, Shown: []domain.AgentShownUsage{}}
	scope := func(q *gorm.DB, alias string) *gorm.DB {
		q = q.Where(alias+".created_at >= ?", since)
		if userID != "" {
			q = q.Where(alias+".user_id = ?", userID)
		}
		return q
	}
	err := scope(r.db.Table("agent_calls c"), "c").
		Select("c.tool, COUNT(*) AS calls, COUNT(DISTINCT c.user_id) AS users, MAX(c.created_at) AS last_at").
		Where("c.tool NOT LIKE ?", domain.AgentHookPrefix+"%").
		Group("c.tool").Order("calls DESC, c.tool").Scan(&out.Tools).Error
	if err != nil {
		return out, err
	}
	// Abierto = la misma persona pidió ese mismo id o ref con una herramienta
	// (no otro hook) poco después de que se lo enseñaran.
	err = scope(r.db.Table("agent_calls s"), "s").
		Select(`substr(s.tool, ?) AS hook, COUNT(*) AS shown,
			COUNT(*) FILTER (WHERE EXISTS (
				SELECT 1 FROM agent_calls o
				WHERE o.user_id = s.user_id AND o.target = s.target AND o.tool NOT LIKE ?
				  AND o.created_at >= s.created_at AND o.created_at <= s.created_at + make_interval(secs => ?))) AS opened`,
			len(domain.AgentHookPrefix)+1, domain.AgentHookPrefix+"%", domain.AgentShownWindow.Seconds()).
		Where("s.tool LIKE ?", domain.AgentHookPrefix+"%").
		Group("s.tool").Order("shown DESC").Scan(&out.Shown).Error
	return out, err
}
