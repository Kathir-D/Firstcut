use eframe::egui;

/// The v2 shell application (D1): wires the engine to the egui UI and owns the
/// process. At M0 it opens an empty dark window on the three native OSes; the
/// `--smoke` flag creates the window, paints a few frames and exits 0 — that is
/// the cross-OS proof the CI matrix runs (docs/v2-plan.md §6, M0).
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let smoke = args.iter().any(|a| a == "--smoke");
    if let Err(err) = run(smoke) {
        eprintln!("firstcut: {err}");
        std::process::exit(1);
    }
}

fn run(smoke: bool) -> Result<(), String> {
    eframe::run_native(
        "firstcut",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_title("Firstcut").with_inner_size([1200.0, 760.0]).with_min_inner_size([640.0, 380.0]),
            ..Default::default()
        },
        Box::new(move |_cc| Ok(Box::new(Shell::new(smoke)))),
    )
    .map_err(|e| format!("eframe failed to create the window: {e}"))
}

struct Shell {
    frames_painted: u32,
    smoke_logged: bool,
    smoke: bool,
}

impl Shell {
    fn new(smoke: bool) -> Self {
        Self { frames_painted: 0, smoke_logged: false, smoke }
    }
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Frame::central_panel(ui.style()).fill(egui::Color32::from_rgb(24, 24, 27)).show(ui, |panel| {
            panel.vertical_centered(|panel| {
                let half = panel.available_height() / 2.0 - 20.0;
                panel.add_space(half.max(0.0));
                panel.heading(egui::RichText::new("Firstcut").color(egui::Color32::from_rgb(232, 232, 236)));
            });
        });
        self.frames_painted += 1;
        // `--smoke`: prove the window exists and paints, then close cleanly.
        if self.smoke && self.frames_painted >= 5 {
            if !self.smoke_logged {
                self.smoke_logged = true;
                eprintln!("smoke: painting frame {}", self.frames_painted);
            }
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
