package http

import (
	"context"
	"fmt"
	"os/signal"
	"syscall"
	"time"

	lg "github.com/guz-studio/cac/backend/internal/core/logger"
	"github.com/guz-studio/cac/backend/internal/core/service"
)

// Cada cuánto da una vuelta el relleno de las carpetas de los commits (ver
// service.BackfillCommitDirs) y de las releases viejas por línea (ver
// service.RelistReleasesByLine). La primera espera lo mismo: no estorba al
// arranque.
const commitDirsTick = time.Minute

// startCommitDirsBackfill: el relleno, en segundo plano. Sin la App para leer
// de GitHub no arranca: no tendría a quién preguntar, y así las pruebas que
// montan las rutas no dejan un reloj suelto.
func startCommitDirsBackfill(gh *service.GitHubService) {
	if !gh.CanRead() {
		return
	}
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	go func() {
		defer stop()
		tick := time.NewTicker(commitDirsTick)
		defer tick.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-tick.C:
				// Un fallo se reintenta en la vuelta siguiente. Primero las
				// releases viejas, que deciden qué commits hacen falta.
				if n, err := gh.RelistReleasesByLine(service.ReleasesRelistedPerRound); err != nil {
					lg.Warn("github: relist releases by line: " + err.Error())
				} else if n > 0 {
					lg.Info(fmt.Sprintf("github: releases relisted by line: %d", n))
				}
				if n, err := gh.BackfillCommitDirs(service.CommitDirsPerRound); err != nil {
					lg.Warn("github: commit dirs backfill: " + err.Error())
				} else if n > 0 {
					lg.Info(fmt.Sprintf("github: commit dirs backfilled: %d commits", n))
				}
			}
		}
	}()
}
