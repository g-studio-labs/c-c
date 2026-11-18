//! `zorite-probe fichero.md`: abre el fichero en zorite-editor.
//! `BLOCKS_BENCH=1` escribe 200 letras en medio (o `BLOCKS_BENCH_KEYS`) y mide; `BLOCKS_HOLD_MS`
//! cierra sola la ventana; `BLOCKS_OUT` es donde van los números.

use std::time::{Duration, Instant};

use gpui_kit::*;
use zorite_probe::Probe;

fn resident_kb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status.lines().find_map(|line| line.strip_prefix("VmRSS:")).and_then(|rest| rest.split_whitespace().next()?.parse().ok()).unwrap_or(0)
}

fn summary(label: &str, samples: &mut [Duration]) -> String {
    if samples.is_empty() {
        return format!("{label}: sin muestras\n");
    }
    samples.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1000.;
    let at = |q: f64| ms(samples[((samples.len() - 1) as f64 * q).round() as usize]);
    format!("{label}: n={} mediana={:.2}ms p95={:.2}ms max={:.2}ms\n", samples.len(), at(0.5), at(0.95), ms(samples[samples.len() - 1]))
}

fn main() {
    let var = |name: &str| std::env::var(name).ok();
    let bench = var("BLOCKS_BENCH").is_some();
    let hold: Option<u64> = var("BLOCKS_HOLD_MS").and_then(|value| value.parse().ok());
    let out = var("BLOCKS_OUT");
    let keys: usize = var("BLOCKS_BENCH_KEYS").and_then(|value| value.parse().ok()).unwrap_or(200);
    let path = std::env::args().nth(1).expect("uso: zorite-probe fichero.md");
    let source = std::fs::read_to_string(&path).expect("no se pudo leer el fichero");

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        zorite_probe::install(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(920.), px(900.)), cx))),
            titlebar: Some(TitlebarOptions { title: Some("block-editor-spike".into()), ..Default::default() }),
            ..Default::default()
        };
        let (window, probe) = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Probe::new(&source, window, cx))).expect("no se pudo abrir la ventana");
        let editor = probe.read(cx).editor.clone();
        let mut report = format!("bytes={} rss_cargado={}kB igual_al_cargar={}\n", source.len(), resident_kb(), editor.read(cx).text() == source);

        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(2500)).await;
            if bench {
                let first = probe.update(cx, |probe, _| std::mem::take(&mut probe.frames));
                let middle = source.len() / 2;
                let middle = (middle..source.len()).find(|ix| source.is_char_boundary(*ix) && source[*ix..].starts_with(". ")).unwrap_or(0);
                editor.update(cx, |editor, cx| editor.set_cursor(middle, cx));
                cx.background_executor().timer(Duration::from_millis(800)).await;
                probe.update(cx, |probe, _| probe.frames.clear());
                let (mut handling, mut frames, mut latency) = (Vec::new(), Vec::new(), Vec::new());
                for ix in 0..keys {
                    let started = Instant::now();
                    let ch = if ix % 7 == 6 { " " } else { ["a", "ñ", "o", "q", "u", "é"][ix % 6] };
                    let _ = window.update(cx, |_, window, cx| editor.update(cx, |editor, cx| editor.replace_text_in_range(None, ch, window, cx)));
                    handling.push(started.elapsed());
                    let before = probe.read_with(cx, |probe, _| probe.frames.len());
                    for _ in 0..40_000 {
                        cx.background_executor().timer(Duration::from_micros(500)).await;
                        if probe.read_with(cx, |probe, _| probe.frames.len()) > before {
                            break;
                        }
                    }
                    latency.push(started.elapsed());
                    frames.extend(probe.update(cx, |probe, _| std::mem::take(&mut probe.frames)));
                }
                report.push_str(&summary("primeros fotogramas", &mut first.clone()));
                report.push_str(&summary("escribir una letra: atender la tecla", &mut handling));
                report.push_str(&summary("escribir una letra: fotograma (render a fin de pintado)", &mut frames));
                report.push_str(&summary("escribir una letra: de la tecla al fin del pintado", &mut latency));
                report.push_str(&format!("rss_final={}kB\n", resident_kb()));
            }
            eprint!("{report}");
            if let Some(out) = &out {
                let _ = std::fs::write(out, &report);
            }
            if let Some(hold) = hold {
                cx.background_executor().timer(Duration::from_millis(hold)).await;
                cx.update(|cx| cx.quit());
            } else if bench {
                cx.update(|cx| cx.quit());
            }
        })
        .detach();
    });
}
