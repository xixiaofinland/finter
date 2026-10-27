// PROTOTYPE for #13, throwaway: not for master.
//
// Question: how can the skim 5.8 picker refresh its rows while open, keep the
// typed query and the highlighted row, and never block on a slow/offline host?
//
//   cargo run --example proto_picker_refresh -- <mode> [--keep] [--refocus] [--inline-slow]
//
// modes (all driven by a timer thread holding `Skim::event_sender()`):
//   reload  Event::Reload + a custom CommandCollector that returns the snapshot
//   events  Event::ClearItems + Event::AppendItems
//   swap    one Action::Custom that swaps the item pool and restarts the matcher
// flags:
//   --keep         SkimOptions::no_clear_if_empty (reload mode)
//   --refocus      after each refresh, move the cursor back to the same row key
//   --inline-slow  reload mode: the collector itself sleeps 3s (fake ssh inline)
//
// A fake world flips agent states every 1.5s, adds/removes a project now and
// then, and a fake host `mini` whose fetch takes 6s and fails every other time
// runs in its own thread, feeding a cache that refreshes only read.
//
// Every refresh appends one line to $PROTO_LOG (default proto.log): the query,
// the focused key before and after, and the list length.

use skim::prelude::*;
use skim::tui::event::ActionCallback;
use skim::tui::App;
use std::io::Write as _;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
enum St {
    Asking,
    Done,
    Working,
    Idle,
}

#[derive(Clone)]
struct Proj {
    name: String,
    st: St,
}

struct Row {
    key: String,
    gen: u64,
    line: String,
}

impl SkimItem for Row {
    fn text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.key)
    }
    fn display(&self, _c: DisplayContext) -> ratatui_core::text::Line<'_> {
        ratatui_core::text::Line::from(self.line.clone())
    }
    // `key@gen` so the refocus step can tell whether the new rows are on screen.
    fn output(&self) -> Cow<'_, str> {
        Cow::Owned(format!("{}@{}", self.key, self.gen))
    }
}

struct World {
    local: Vec<Proj>,
    // Last good fetch of the fake host, plus whether the last try failed.
    host_cache: Vec<Proj>,
    host_offline: bool,
    gen: u64,
}

fn icon(st: St) -> &'static str {
    match st {
        St::Asking => "❓",
        St::Done => "✅",
        St::Working => "⚙ ",
        St::Idle => "  ",
    }
}

fn rank(st: St) -> u8 {
    st as u8
}

/// Rows sorted like finter: asking, done, working, idle.
fn snapshot(w: &mut World) -> Vec<Arc<dyn SkimItem>> {
    w.gen += 1;
    let gen = w.gen;
    let mut all: Vec<(String, St)> = w
        .local
        .iter()
        .map(|p| (p.name.clone(), p.st))
        .chain(
            w.host_cache
                .iter()
                .map(|p| (format!("mini/{}", p.name), p.st)),
        )
        .collect();
    all.sort_by_key(|(_, st)| rank(*st));
    let off = if w.host_offline { " [mini ✕]" } else { "" };
    all.into_iter()
        .map(|(key, st)| {
            Arc::new(Row {
                line: format!("{} {key:<14} gen={gen}{off}", icon(st)),
                key,
                gen,
            }) as Arc<dyn SkimItem>
        })
        .collect()
}

fn split(out: &str) -> (String, u64) {
    let (k, g) = out.rsplit_once('@').unwrap_or((out, "0"));
    (k.to_string(), g.parse().unwrap_or(0))
}

fn focused(app: &App) -> Option<(String, u64)> {
    app.item_list.selected().map(|m| split(&m.item.output()))
}

fn log(line: String) {
    let path = std::env::var("PROTO_LOG").unwrap_or_else(|_| "proto.log".into());
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{line}");
    }
}

struct CacheCollector {
    world: Arc<Mutex<World>>,
    inline_slow: bool,
}

impl CommandCollector for CacheCollector {
    fn invoke(
        &mut self,
        _cmd: &str,
        _components_to_stop: Arc<AtomicUsize>,
    ) -> (SkimItemReceiver, Sender<i32>) {
        if self.inline_slow {
            // What happens if the reload path itself waited on ssh.
            std::thread::sleep(Duration::from_secs(3));
        }
        let (tx, rx): (SkimItemSender, SkimItemReceiver) = unbounded();
        let _ = tx.send(snapshot(&mut self.world.lock().unwrap()));
        let (tx_stop, _rx_stop) = unbounded();
        (rx, tx_stop)
    }
}

/// Wanted focus: key and the generation that must be on screen first.
type Want = Arc<Mutex<Option<(String, u64, Instant)>>>;

fn refocus_action(want: Want, tx: tokio::sync::mpsc::Sender<Event>, tries: u32) -> Event {
    Event::Action(Action::Custom(ActionCallback::new_sync(
        move |app: &mut App| {
            let Some((key, gen, started)) = want.lock().unwrap().clone() else {
                return Ok(vec![]);
            };
            let on_screen = focused(app).map(|(_, g)| g).unwrap_or(0);
            if on_screen < gen {
                // New rows not rendered yet: try again shortly, off the UI thread.
                if tries < 200 {
                    let (want, tx) = (want.clone(), tx.clone());
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(10));
                        let _ = tx.try_send(refocus_action(want.clone(), tx.clone(), tries + 1));
                    });
                }
                return Ok(vec![]);
            }
            *want.lock().unwrap() = None;
            let before = focused(app).map(|f| f.0);
            let mut found = before.as_deref() == Some(key.as_str());
            if !found {
                app.item_list.jump_to_first();
                for _ in 0..app.item_list.count() {
                    if focused(app).is_some_and(|(k, _)| k == key) {
                        found = true;
                        break;
                    }
                    app.item_list.select_next();
                }
                if !found {
                    app.item_list.jump_to_first();
                }
            }
            log(format!(
            "  refocus gen={gen} want={key} landed_on={:?} found={found} tries={tries} after={:?}",
            focused(app).map(|f| f.0),
            started.elapsed()
        ));
            Ok(vec![])
        },
    )))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().cloned().unwrap_or_else(|| "swap".into());
    let has = |f: &str| args.iter().any(|a| a == f);
    let (keep, refocus, inline_slow) = (has("--keep"), has("--refocus"), has("--inline-slow"));

    let names = [
        "alpha", "beta", "gamma", "delta", "eps", "zeta", "eta", "theta",
    ];
    let world = Arc::new(Mutex::new(World {
        local: names
            .iter()
            .map(|n| Proj {
                name: n.to_string(),
                st: St::Idle,
            })
            .collect(),
        host_cache: vec![],
        host_offline: true,
        gen: 0,
    }));

    let mut builder = SkimOptionsBuilder::default();
    builder.height("100%").multi(false).header(format!(
        "PROTO #13 mode={mode} keep={keep} refocus={refocus} inline_slow={inline_slow}"
    ));
    if keep {
        builder.no_clear_if_empty(true);
    }
    let mut options = builder.build().unwrap();
    options.cmd_collector = Rc::new(RefCell::new(CacheCollector {
        world: world.clone(),
        inline_slow,
    })) as Rc<RefCell<dyn CommandCollector>>;
    if mode == "reload" {
        // Gives the reader a command, so the initial rows come from the collector too.
        options.cmd = Some("proto".into());
    }

    let source = if mode == "reload" {
        None
    } else {
        let (tx, rx): (SkimItemSender, SkimItemReceiver) = unbounded();
        let _ = tx.send(snapshot(&mut world.lock().unwrap()));
        Some(rx)
    };

    let mut skim = Skim::init(options, source).unwrap();
    skim.start();
    skim.init_tui().unwrap();
    let tx = skim.event_sender();

    // ── fake world: local states flip, projects come and go
    {
        let world = world.clone();
        std::thread::spawn(move || {
            let mut i = 0usize;
            loop {
                std::thread::sleep(Duration::from_millis(1500));
                let mut w = world.lock().unwrap();
                i += 1;
                let n = w.local.len();
                let p = &mut w.local[(i * 3) % n];
                p.st = match p.st {
                    St::Idle => St::Working,
                    St::Working => St::Done,
                    St::Done => St::Asking,
                    St::Asking => St::Idle,
                };
                if i % 7 == 0 {
                    w.local.push(Proj {
                        name: format!("new{i}"),
                        st: St::Working,
                    });
                }
                if i % 11 == 0 && w.local.len() > 4 {
                    w.local.remove(1);
                }
            }
        });
    }

    // ── fake host: slow fetch (6s), offline every other try; only fills the cache
    {
        let world = world.clone();
        std::thread::spawn(move || {
            let mut ok = false;
            loop {
                std::thread::sleep(Duration::from_secs(6));
                ok = !ok;
                let mut w = world.lock().unwrap();
                w.host_offline = !ok;
                if ok {
                    w.host_cache = vec![
                        Proj {
                            name: "recap".into(),
                            st: St::Working,
                        },
                        Proj {
                            name: "finter".into(),
                            st: St::Done,
                        },
                    ];
                }
            }
        });
    }

    // ── refresh timer
    {
        let world = world.clone();
        let tx = tx.clone();
        let mode = mode.clone();
        let want: Want = Arc::new(Mutex::new(None));
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(2));
            let started = Instant::now();
            // Record query and focus as the UI sees them right before the refresh.
            let (want2, gen_next) = (want.clone(), world.lock().unwrap().gen + 1);
            let refocus_on = refocus;
            let _ = tx.try_send(Event::Action(Action::Custom(ActionCallback::new_sync(
                move |app: &mut App| {
                    let f = focused(app);
                    log(format!(
                        "refresh gen={gen_next} query={:?} focus_before={:?} count={}",
                        app.input.value,
                        f.as_ref().map(|f| &f.0),
                        app.item_list.count()
                    ));
                    if refocus_on {
                        if let Some((k, _)) = f {
                            *want2.lock().unwrap() = Some((k, gen_next, started));
                        }
                    }
                    Ok(vec![])
                },
            ))));
            match mode.as_str() {
                "reload" => {
                    let _ = tx.try_send(Event::Reload("proto".into()));
                }
                "events" => {
                    let items = snapshot(&mut world.lock().unwrap());
                    let _ = tx.try_send(Event::ClearItems);
                    let _ = tx.try_send(Event::AppendItems(items));
                }
                _ => {
                    let items = snapshot(&mut world.lock().unwrap());
                    let _ = tx.try_send(Event::Action(Action::Custom(ActionCallback::new_sync(
                        move |app: &mut App| {
                            app.item_pool.clear();
                            app.item_pool.append(items.clone());
                            app.restart_matcher(true);
                            Ok(vec![])
                        },
                    ))));
                }
            }
            if refocus {
                let _ = tx.try_send(refocus_action(want.clone(), tx.clone(), 0));
            }
            // Log every 100ms for 600ms what the user sees as focused.
            for _ in 0..6 {
                std::thread::sleep(Duration::from_millis(100));
                let t = started.elapsed();
                let _ = tx.try_send(Event::Action(Action::Custom(ActionCallback::new_sync(
                    move |app: &mut App| {
                        log(format!(
                            "  +{:>4}ms focus={:?} count={}",
                            t.as_millis(),
                            focused(app),
                            app.item_list.count()
                        ));
                        Ok(vec![])
                    },
                ))));
            }
        });
    }

    let rt = tokio::runtime::Runtime::new().unwrap();
    let out = rt
        .block_on(async {
            skim.enter().await.map_err(|e| e.to_string())?;
            skim.run().await.map_err(|e| e.to_string())?;
            Ok::<_, Box<dyn std::error::Error>>(skim)
        })
        .map(|s| s.output());
    match out {
        Ok(o) => println!(
            "selected={:?} query={:?} abort={}",
            o.selected_items.first().map(|i| i.output().to_string()),
            o.query,
            o.is_abort
        ),
        Err(e) => eprintln!("error: {e}"),
    }
}
