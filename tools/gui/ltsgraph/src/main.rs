// This is a GUI application
#![windows_subsystem = "windows"]
// Suppress unreachable_pub for Slint-generated code (binary crate has no external API).
#![allow(unreachable_pub)]

slint::include_modules!();

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Instant;

use clap::Parser;
use log::debug;
use log::info;
use log::warn;
use slint::Image;
use slint::Rgba8Pixel;
use slint::SharedPixelBuffer;
use slint::invoke_from_event_loop;
use slint::quit_event_loop;

use merc_io::LargeFormatter;
use merc_lts::LTS;
use merc_lts::LabelledTransitionSystem;
use merc_lts::LtsFormat;
use merc_lts::apply_lts;
use merc_lts::guess_lts_format_from_extension;
use merc_lts::read_explicit_lts;
use merc_ltsgraph_lib::GraphLayout;
use merc_ltsgraph_lib::Viewer;
use merc_tools::VerbosityFlag;
use merc_tools::Version;
use merc_tools::init_console;
use merc_tools::report_error;
use merc_utilities::MercError;
use merc_utilities::Timing;

use merc_ltsgraph::PauseableThread;
use merc_ltsgraph::RenderSettings;
use merc_ltsgraph::Renderer;
use merc_ltsgraph::ViewerType;
use merc_ltsgraph::init_wgpu;
use merc_ltsgraph::show_error_dialog;

/// A GUI tool to view labelled transition systems.
#[derive(Parser, Debug)]
#[command(about = "A labelled transition system GUI tool")]
pub struct Cli {
    #[arg(
        long,
        global = true,
        default_value_t = false,
        help = "Print the version of this tool"
    )]
    version: bool,

    #[command(flatten)]
    verbosity: VerbosityFlag,

    /// Path to the labelled transition system to load on startup.
    #[arg(value_name = "FILE")]
    labelled_transition_system: Option<String>,

    /// Explicitly specify the LTS format.
    #[arg(long)]
    lts_format: Option<LtsFormat>,

    /// Change the viewer to CPU or GPU rendering.
    #[arg(long, default_value_t = ViewerType::Cpu, value_enum)]
    viewer: ViewerType,

    /// Print timing information after loading an LTS.
    #[arg(long)]
    timings: bool,
}

/// The graph layout, viewer state and cached LTS for the LTS currently being displayed.
///
/// These three must always be replaced together when a new LTS is loaded: the render thread reads
/// `viewer` together with the renderer it rebuilds from `lts`/`reload_lts`, and the layout thread
/// reads `graph_layout` together with `viewer`. Keeping all four fields behind a single `Mutex`
/// (rather than one `Mutex` per field, as `graph_layout`/`viewer`/`lts` used to be) guarantees that
/// a reader never observes a `Viewer` paired with a `GraphLayout` or renderer-cached LTS taken from
/// a different load, which would let it index one with the other's (differently-sized) indices and
/// panic.
struct ReloadState {
    graph_layout: Option<GraphLayout>,
    viewer: Option<Viewer>,
    lts: Option<Arc<LabelledTransitionSystem<String>>>,
    /// Set when `lts` has changed and the render thread's cached renderer still needs rebuilding.
    reload_lts: bool,
}

/// Contains all the GUI related state information, both the graph layout and the viewer state.
struct State {
    reload: Mutex<ReloadState>,
    canvas: Arc<Mutex<SharedPixelBuffer<Rgba8Pixel>>>,
    /// Whether the layout simulation is enabled, so layout-parameter changes only restart it while running.
    simulation_enabled: AtomicBool,
}

#[derive(Clone, Default)]
pub struct GuiSettings {
    // Layout related settings
    pub handle_length: f32,
    pub repulsion_strength: f32,
    pub delta: f32,

    // View related settings
    pub width: u32,
    pub height: u32,
    pub state_radius: f32,
    pub label_text_size: f32,
    pub draw_action_labels: bool,

    pub zoom_level: f32,
    pub view_x: f32,
    pub view_y: f32,
}

impl GuiSettings {
    pub fn new() -> GuiSettings {
        GuiSettings {
            width: 1,
            height: 1,
            zoom_level: 1.0,
            ..Default::default()
        }
    }

    /// Converts the GUI settings to the render settings, which are used for
    /// rendering the graph.
    pub fn to_render_settings(&self) -> RenderSettings {
        RenderSettings {
            width: self.width,
            height: self.height,
            state_radius: self.state_radius,
            label_text_size: self.label_text_size,
            draw_action_labels: self.draw_action_labels,
            zoom_level: self.zoom_level,
            view_x: self.view_x,
            view_y: self.view_y,
        }
    }
}

// Initialize a tokio runtime for async calls
#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    report_error(run().await)
}

async fn run() -> Result<(), MercError> {
    // Attach the standard output to the command line.
    let _console = init_console()?;

    let cli = Cli::parse();

    env_logger::Builder::new()
        .filter_level(cli.verbosity.log_level_filter())
        .parse_default_env()
        .init();

    if cli.version {
        eprintln!("{}", Version);
        return Ok(());
    }

    let wgpu = if cli.viewer == ViewerType::Gpu {
        // Initialize wgpu for GPU rendering
        Some(init_wgpu().await?)
    } else {
        None
    };

    // Stores the shared state of the GUI components.
    let settings = Arc::new(Mutex::new(GuiSettings::new()));
    let state = Arc::new(State {
        reload: Mutex::new(ReloadState {
            graph_layout: None,
            viewer: None,
            lts: None,
            reload_lts: false,
        }),
        canvas: Arc::new(Mutex::new(SharedPixelBuffer::new(1, 1))),
        simulation_enabled: AtomicBool::new(true),
    });

    // Initialize the GUI, but show it later.
    let app = Application::new()?;
    {
        let app_weak = app.as_weak();
        let settings = settings.clone();

        app.on_settings_changed(move || {
            // Request the settings for the next simulation tick.
            if let Some(app) = app_weak.upgrade() {
                let mut settings = settings.lock().unwrap();
                settings.handle_length = app.global::<Settings>().get_handle_length();
                settings.repulsion_strength = app.global::<Settings>().get_repulsion_strength();
                settings.delta = app.global::<Settings>().get_timestep();
                settings.state_radius = app.global::<Settings>().get_state_radius();

                settings.draw_action_labels = app.global::<Settings>().get_draw_action_labels();
                settings.zoom_level = app.global::<Settings>().get_zoom_level();
                settings.view_x = app.global::<Settings>().get_view_x();
                settings.view_y = app.global::<Settings>().get_view_y();
                settings.label_text_size = app.global::<Settings>().get_label_text_height();
            }
        });
    };

    // Trigger it once to set the default values.
    app.invoke_settings_changed();

    // Render the view continuously, but only update the canvas when necessary
    let render_handle = {
        let state = state.clone();
        let app_weak: slint::Weak<Application> = app.as_weak();
        let settings = settings.clone();
        let settings_init = settings.clone();
        let viewer_type = cli.viewer.clone();

        Arc::new(PauseableThread::new(
            "ltsgraph canvas worker",
            move || {
                let initial = settings_init.lock().unwrap().clone();
                Ok(Renderer::new(
                    viewer_type.clone(),
                    wgpu.clone(),
                    initial.width,
                    initial.height,
                ))
            },
            move |renderer| {
                let render_settings = settings.lock().unwrap().to_render_settings();

                // Held for the rest of this iteration so the renderer rebuild (from `lts`) and the
                // subsequent render (using `viewer`) always see a pair from the same load.
                let mut reload_state = state.reload.lock().unwrap();

                if reload_state.reload_lts {
                    info!("Creating the renderer");
                    if let Some(lts) = reload_state.lts.as_ref() {
                        renderer.reload(lts.clone(), &render_settings)?;
                    }
                    reload_state.reload_lts = false;
                }

                if let Some(viewer) = reload_state.viewer.as_mut() {
                    let start = Instant::now();
                    renderer.render(viewer, &render_settings, &state.canvas)?;
                    debug!(
                        "Rendering step ({} by {}) took {} ms",
                        render_settings.width,
                        render_settings.height,
                        (Instant::now() - start).as_millis()
                    );
                } else {
                    // If we are not rendering the graph, we still need to ensure the canvas is initialized.
                    let mut canvas = state.canvas.lock().unwrap();
                    if canvas.width() != render_settings.width || canvas.height() != render_settings.height {
                        *canvas = SharedPixelBuffer::<Rgba8Pixel>::new(render_settings.width, render_settings.height);
                    }
                }

                // Request the canvas to be updated.
                let app_weak = app_weak.clone();
                invoke_from_event_loop(move || {
                    if let Some(app) = app_weak.upgrade() {
                        // Update the canvas
                        app.global::<Settings>()
                            .set_refresh(!app.global::<Settings>().get_refresh());
                    };
                })
                .unwrap();

                Ok(false)
            },
            |e: &MercError| {
                let message = e.to_string();
                let _ = invoke_from_event_loop(move || {
                    let _ = show_error_dialog("Rendering error", &message);
                });
            },
        )?)
    };

    // Run the graph layout algorithm in a separate thread to avoid blocking the UI.
    let layout_handle = {
        let state = state.clone();
        let settings = settings.clone();
        let render_handle = render_handle.clone();

        Arc::new(PauseableThread::new(
            "ltsgraph layout worker",
            || Ok(()),
            move |_| {
                let mut is_stable = true;

                // Held across both the layout step and the viewer copy below so the layout thread
                // never copies a `GraphLayout` into a `Viewer` from a different (already-reloaded,
                // differently-sized) load, or vice versa.
                let mut reload_state = state.reload.lock().unwrap();
                let ReloadState {
                    graph_layout, viewer, ..
                } = &mut *reload_state;

                if let Some(layout) = graph_layout.as_mut() {
                    // Read the settings and free the lock since otherwise the callback above blocks.
                    let settings = settings.lock().unwrap().clone();

                    let start = Instant::now();
                    is_stable = layout.update(settings.handle_length, settings.repulsion_strength, settings.delta);
                    if is_stable {
                        info!("Layout is stable!");
                    }

                    let duration = Instant::now() - start;
                    debug!("Layout step took {} ms", duration.as_millis());

                    // Copy layout into the view.
                    if let Some(viewer) = viewer.as_mut() {
                        viewer.update(layout);
                    }

                    // Request a redraw (if not already in progress).
                    render_handle.resume();
                }

                // If stable pause the thread.
                Ok(!is_stable)
            },
            |e: &MercError| {
                let message = e.to_string();
                let _ = invoke_from_event_loop(move || {
                    let _ = show_error_dialog("Layout error", &message);
                });
            },
        )?)
    };

    // Load an LTS from the given path and updates the state.
    let load_lts = {
        let state = state.clone();
        let layout_handle = layout_handle.clone();
        let render_handle = render_handle.clone();
        let timings = cli.timings;

        move |path: &Path, format: Option<LtsFormat>| -> Result<(), MercError> {
            debug!("Loading LTS {} ...", path.to_string_lossy());

            let format = guess_lts_format_from_extension(path, format).ok_or("Unknown LTS file format.")?;
            let mut timing = Timing::new();
            match read_explicit_lts(path, format, &mut timing) {
                Ok(lts) => {
                    // Ensure that the labels are strings, such that they can displayed.
                    let lts: Arc<LabelledTransitionSystem<String>> =
                        apply_lts!(lts, (), |lts, _data_spec, _| -> Result<_, MercError> {
                            Ok(Arc::new(lts.relabel(|label| Ok(label.to_string()))?))
                        })?;

                    info!(
                        "Loaded lts with {} states and {} transitions",
                        LargeFormatter(lts.num_of_states()),
                        LargeFormatter(lts.num_of_transitions())
                    );

                    // Create the layout and viewer separately to make the initial state sensible.
                    let layout = GraphLayout::new(lts.clone());
                    let mut viewer = Viewer::new(lts.clone());

                    // Update view to the initial layout.
                    viewer.update(&layout);

                    // Replace the viewer, layout and cached LTS as a single atomic update: taking
                    // the lock once, rather than once per field, is what prevents a background
                    // thread from ever observing a partially-updated combination of the three (see
                    // `ReloadState`'s documentation).
                    {
                        let mut reload_state = state.reload.lock().unwrap();
                        reload_state.viewer = Some(viewer);
                        reload_state.graph_layout = Some(layout);
                        reload_state.lts = Some(lts);
                        reload_state.reload_lts = true;
                    }

                    // Enable the layout and rendering threads.
                    layout_handle.resume();
                    render_handle.resume();

                    if timings {
                        timing.print();
                    }
                    Ok(())
                }
                Err(x) => show_error_dialog("Failed to load LTS!", &format!("{x}")),
            }
        }
    };

    // When the simulation is toggled enable the layout thread.
    {
        let layout_handle = layout_handle.clone();
        let state = state.clone();
        app.on_run_simulation(move |enabled| {
            state.simulation_enabled.store(enabled, Ordering::Relaxed);
            if enabled {
                layout_handle.resume();
            } else {
                layout_handle.pause();
            }
        })
    }

    // When a layout parameter changes, restart the layout simulation so the
    // change takes effect, but only while the simulation is enabled.
    {
        let layout_handle = layout_handle.clone();
        let state = state.clone();
        app.on_request_relayout(move || {
            if state.simulation_enabled.load(Ordering::Relaxed) {
                layout_handle.resume();
            }
        })
    }

    // Simply return the current canvas, can be updated in the meantime.
    {
        let state = state.clone();
        let settings = settings.clone();
        let render_handle = render_handle.clone();

        app.on_update_canvas(move |width, height, _| {
            let mut settings = settings.lock().unwrap();
            settings.width = width as u32;
            settings.height = height as u32;

            let canvas = state.canvas.lock().unwrap().clone();
            if canvas.width() != settings.width || canvas.height() != settings.height {
                // Request another redraw when the size has changed.
                debug!(
                    "Canvas size changed from {}x{} to {width}x{height}",
                    canvas.width(),
                    canvas.height()
                );
                render_handle.resume();
            }

            debug!("Updating canvas");
            Image::from_rgba8_premultiplied(canvas)
        });
    }

    // If a redraw was requested resume the render thread.
    {
        let render_handle = render_handle.clone();
        app.on_request_redraw(move || {
            render_handle.resume();
        })
    }

    // Open the file dialog and load another LTS if necessary.
    {
        let load_lts = load_lts.clone();
        app.on_open_filedialog(move || {
            let load_lts = load_lts.clone();

            invoke_from_event_loop(move || {
                slint::spawn_local(async move {
                    if let Some(handle) = rfd::AsyncFileDialog::new()
                        .add_filter("", &["aut", "lts", "bcg"])
                        .pick_file()
                        .await
                        && load_lts(handle.path(), cli.lts_format).is_err()
                    {
                        warn!("Failed to load LTS from file dialog.");
                    }
                })
                .unwrap();
            })
            .unwrap();
        });
    }

    // Focus on the graph
    {
        let settings = settings.clone();
        let state = state.clone();
        let render_handle = render_handle.clone();
        let app_weak = app.as_weak();
        let settings = settings.clone();

        app.on_focus_view(move || {
            if let Some(app) = app_weak.upgrade()
                && let Some(viewer) = state.reload.lock().unwrap().viewer.as_ref()
            {
                debug!("Centering view on graph.");

                let center = viewer.center();

                // Change the view to show the LTS in full.
                app.global::<Settings>().set_view_x(center.x);
                app.global::<Settings>().set_view_y(center.y);

                let mut settings = settings.lock().unwrap();
                settings.view_x = center.x;
                settings.view_y = center.y;

                render_handle.resume();
            }
        });
    }

    // Show the about dialog
    {
        app.on_show_about_dialog(|| {
            let dialog = AboutDialog::new().expect("Creating the AboutDialog failed");
            dialog.show().expect("Showing the AboutDialog failed");
        })
    }

    app.on_quit(move || {
        // Stop the layout and quit.
        let _ = quit_event_loop();
    });

    // Loads the LTS given on the command line.
    if let Some(path) = &cli.labelled_transition_system {
        load_lts(Path::new(path), cli.lts_format)?;
    }

    app.run()?;

    // Stop the layout and quit.
    layout_handle.stop();
    render_handle.stop();

    Ok(())
}
