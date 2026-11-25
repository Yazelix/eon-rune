//! Native close witness driven by a private Wayland compositor (#174).

#[cfg(target_os = "linux")]
mod native {
    use std::cell::RefCell;

    use rune::{App, AppSpec, CloseResponse, Key, KeyEvent, Tx, WindowSpec, text};

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    enum Case {
        #[default]
        AppAccepts,
        KeyedProgrammatic,
        KeyedNeighbor,
        DefaultClose,
    }

    #[derive(Default)]
    struct State {
        case: Case,
        asks: u8,
    }

    #[derive(Default)]
    struct Receipt {
        asks: u8,
        drew_confirmation: bool,
        input: u8,
        closed: u8,
        neighbor_closed: u8,
    }

    thread_local! { static RECEIPT: RefCell<Receipt> = RefCell::new(Receipt::default()); }

    fn on_close_request(state: &mut State, _: &mut Tx) -> CloseResponse {
        state.asks += 1;
        RECEIPT.with(|receipt| receipt.borrow_mut().asks += 1);
        println!("close-request {}", state.asks);
        if state.asks == 1 { CloseResponse::Defer } else { CloseResponse::Close }
    }

    fn on_key(state: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
        let Key::Text(key) = &event.key else { return false };
        match key.as_str() {
            "k" if state.asks == 1 => {
                RECEIPT.with(|receipt| receipt.borrow_mut().input += 1);
                println!("input-after-defer");
            }
            "p" if state.case == Case::KeyedProgrammatic && state.asks == 1 => tx.close_window(),
            "o" if state.case == Case::KeyedNeighbor && state.asks == 1 => {
                let spec = AppSpec::new("Close probe neighbor", (320.0, 160.0), || (), |_, _| text("Neighbor"));
                drop(rune::open_window(WindowSpec::new("neighbor", spec)
                    .on_ready(|_| println!("neighbor-ready"))
                    .on_close(|_, _| {
                        RECEIPT.with(|receipt| receipt.borrow_mut().neighbor_closed += 1);
                        println!("neighbor-closed");
                    })));
            }
            _ => return false,
        }
        true
    }

    pub(super) fn run() {
        let Ok(mode) = std::env::var("RUNE_CLOSE_REQUEST_CASE") else { return };
        assert!(std::env::var_os("WAYLAND_DISPLAY").is_some());
        assert!(std::env::var_os("DISPLAY").is_none());
        rune::tooling::set_app_id("org.rune.CloseRequestProbe");
        let case = match mode.as_str() {
            "app" => Case::AppAccepts,
            "programmatic" => Case::KeyedProgrammatic,
            "neighbor" => Case::KeyedNeighbor,
            "default" => Case::DefaultClose,
            _ => panic!("unknown close case {mode}"),
        };
        let mut spec = AppSpec::new("Close probe", (400.0, 240.0), State::default, |state, _| {
            text(if state.asks == 0 { "Ready" } else { "Confirm close?" })
        });
        spec.on_key = Some(on_key);
        let app = App::new(spec).args(Vec::new()).init(move || State { case, asks: 0 })
            .on_ready(|_| println!("primary-ready"))
            .on_frame_drawn(|runtime| {
                RECEIPT.with(|receipt| {
                    let mut receipt = receipt.borrow_mut();
                    if runtime.state.asks == 1 && !receipt.drew_confirmation {
                        receipt.drew_confirmation = true;
                        println!("confirmation-drawn");
                    }
                });
            })
            .on_close(|_, _| {
                RECEIPT.with(|receipt| receipt.borrow_mut().closed += 1);
                println!("primary-closed");
            });
        rune::orderly_exit();
        match case {
            Case::AppAccepts => app.on_close_request(on_close_request).run(),
            Case::DefaultClose => app.run(),
            Case::KeyedProgrammatic | Case::KeyedNeighbor => rune::run_windows_with_args(
                rune::RunOptions::default(), Vec::new(), move |_| {
                    [WindowSpec::from_app("primary", app).on_close_request(on_close_request)]
                },
            ),
        }
        RECEIPT.with(|receipt| {
            let receipt = receipt.borrow();
            let asks = match case { Case::DefaultClose => 0, Case::KeyedProgrammatic => 1, _ => 2 };
            assert_eq!(receipt.asks, asks);
            assert_eq!(receipt.drew_confirmation, asks > 0);
            assert_eq!(receipt.input, u8::from(asks > 0));
            assert_eq!(receipt.closed, 1);
            assert_eq!(receipt.neighbor_closed, u8::from(case == Case::KeyedNeighbor));
        });
        println!("wayland-close-request: pass {mode}");
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    native::run();
}
