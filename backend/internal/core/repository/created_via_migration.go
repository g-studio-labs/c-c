package repository

import (
	"fmt"

	lg "github.com/guz-studio/cac/backend/internal/core/logger"
	"gorm.io/gorm"
)

// Los mensajes de canal y de directo guardan con qué vía se crearon
// (`created_via`), aparte de `via`, que desde command-and-control-547 sigue a
// la última corrección. Retirar con un token decide por la de creación.
//
// La columna nace aquí y no por `AutoMigrate` para rellenarla **una sola vez**
// con el `via` de cada fila: hasta ahora corregir no tocaba `via`, así que hoy
// es la de creación. Rellenarla en cada arranque sería el fallo otra vez:
// un mensaje tecleado a mano y corregido por un agente tiene `via = mcp`.
// Por eso la idempotencia la da el esquema: si la columna ya está, está hecho.
var createdViaTables = []string{"chat_messages", "dm_messages"}

func addCreatedVia(db *gorm.DB) {
	const lockKey = 0x3D_11_26_47
	if err := db.Exec(`SELECT pg_advisory_lock(?)`, lockKey).Error; err != nil {
		panic("created_via migration: cannot take the lock: " + err.Error())
	}
	defer db.Exec(`SELECT pg_advisory_unlock(?)`, lockKey)

	for _, table := range createdViaTables {
		var tableFound, columnFound string
		db.Raw(`SELECT table_name FROM information_schema.tables WHERE table_name = ?`, table).Scan(&tableFound)
		if tableFound == "" {
			// Base recién creada: `AutoMigrate` la hace ya con la columna.
			continue
		}
		db.Raw(`SELECT column_name FROM information_schema.columns
		        WHERE table_name = ? AND column_name = 'created_via'`, table).Scan(&columnFound)
		if columnFound != "" {
			continue
		}
		err := db.Transaction(func(tx *gorm.DB) error {
			if err := tx.Exec(fmt.Sprintf(
				`ALTER TABLE %s ADD COLUMN created_via varchar(10) NOT NULL DEFAULT ''`, table)).Error; err != nil {
				return err
			}
			return tx.Exec(fmt.Sprintf(`UPDATE %s SET created_via = via`, table)).Error
		})
		if err != nil {
			panic("created_via migration: " + table + ": " + err.Error())
		}
		lg.Info("created_via migration: " + table + ".created_via added from via")
	}
}
