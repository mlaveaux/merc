// This is a GUI application
#![windows_subsystem = "windows"]

slint::include_modules!();

use std::ops::Deref;
use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use clap::Parser;
use clap::ValueEnum;
use femtovg::renderer::WGPURenderer;
use femtovg::Canvas;
use log::debug;
use log::info;
use log::warn;
use slint::invoke_from_event_loop;
use slint::quit_event_loop;
use slint::Image;
use slint::Rgba8Pixel;
use slint::SharedPixelBuffer;
use wgpu::TextureDescriptor;
use wgpu::TextureFormat;
use wgpu::TextureUsages;

use merc_io::LargeFormatter;
use merc_lts::apply_lts;
use merc_lts::guess_lts_format_from_extension;
use merc_lts::read_explicit_lts;
use merc_lts::LabelledTransitionSystem;
use merc_lts::LtsFormat;
use merc_lts::LTS;
use merc_ltsgraph_lib::FemtovgRenderer;
use merc_ltsgraph_lib::GraphLayout;
use merc_ltsgraph_lib::SkiaRenderer;
use merc_ltsgraph_lib::Viewer;
use merc_tools::console;
use merc_tools::verbosity::VerbosityFlag;
use merc_tools::Version;
use merc_utilities::MercError;
use merc_utilities::Timing;

use merc_ltsgraph::init_wgpu;
use merc_ltsgraph::show_error_dialog;
use merc_ltsgraph::PauseableThread;

/// Aligns a number up to the next multiple of the given alignment.
pub const fn align_up(num: u32, align: u32) -> u32 {
    ((num) + ((align) - 1)) & !((align) - 1)
}

#[derive(Clone, Debug, ValueEnum, PartialEq, Eq, PartialOrd, Ord)]
enum ViewerType {
    /// Uses tiny-skia to render the graph on the CPU
    Cpu,
    /// Uses femtovg to render the graph on the GPU, with wgpu
    Gpu,
}

#[derive(Parser, Debug)]
#[command(name = "Maurice Laveaux", about = "A lts viewing tool")]
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

    /// Path to the labelled transition system to load
    #[arg(value_name = "FILE")]
    labelled_transition_system: Option<String>,

    /// Explicitly specify the LTS format
    #[arg(long)]
    lts_format: Option<LtsFormat>,

    #[arg(default_value_t = ViewerType::Cpu, value_enum)]
    viewer: ViewerType,
}

/// Contains all the GUI related state information, both the graph layout and the viewer state.
struct State {
    graph_layout: Mutex<Option<GraphLayout>>,
    viewer: Mutex<Option<Viewer>>,
    canvas: Mutex<SharedPixelBuffer<Rgba8Pixel>>,
    lts: Mutex<Option<Arc<LabelledTransitionSystem<String>>>>,
    reload_lts: AtomicBool,
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
            view_x: 500.0,
            view_y: 500.0,
            ..Default::default()
        }
    }
}

// Initialize a tokio runtime for async calls
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<ExitCode, MercError> {
    // Attach the standard output to the command line.
    let _console = console::init()?;

    let cli = Cli::parse();

    env_logger::Builder::new()
        .filter_level(cli.verbosity.log_level_filter())
        .parse_default_env()
        .init();

    if cli.version {
        eprintln!("{}", Version);
        return Ok(ExitCode::SUCCESS);
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
        graph_layout: Mutex::new(None),
        viewer: Mutex::new(None),
        canvas: Mutex::new(SharedPixelBuffer::new(1, 1)),
        reload_lts: AtomicBool::new(false),
        lts: Mutex::new(None),
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

        /// Local information required for the femtovg renderer.
        struct FemtovgInfo {
            renderer: FemtovgRenderer,
            canvas: Canvas<WGPURenderer>,
            texture: wgpu::Texture,
            buffer: Arc<wgpu::Buffer>,
        }

        Arc::new(PauseableThread::new(
            "ltsgraph canvas worker",
            move || {
                let settings = settings_init.lock().unwrap().clone();

                Ok((
                    None::<SkiaRenderer>,
                    None::<FemtovgInfo>,
                    SharedPixelBuffer::<Rgba8Pixel>::new(settings.width, settings.height),
                ))
            },
            move |(skia_renderer, femtovg_info, pixel_buffer)| {
                let settings = settings.lock().unwrap().clone();

                if state.reload_lts.load(Ordering::Relaxed) {
                    info!("Creating the renderer");
                    if let Some(lts) = state.lts.lock().unwrap().as_ref() {
                        *skia_renderer = Some(SkiaRenderer::new(lts.clone()));

                        *femtovg_info = if let Some((device, queue)) = &wgpu {
                            let gpu_renderer = WGPURenderer::new(device.clone(), queue.clone());
                            let canvas = Canvas::new(gpu_renderer)?;

                            // Create the texture and buffer for the femtovg renderer
                            let texture = device.create_texture(&TextureDescriptor {
                                label: Some("ltsgraph canvas texture"),
                                size: wgpu::Extent3d {
                                    width: settings.width,
                                    height: settings.height,
                                    depth_or_array_layers: 1,
                                },
                                mip_level_count: 1,
                                sample_count: 1,
                                dimension: wgpu::TextureDimension::D2,
                                format: TextureFormat::Rgba8UnormSrgb,
                                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
                                view_formats: &[],
                            });

                            // This buffer is used to copy the rendered image to the texture.
                            let buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
                                label: Some("ltsgraph canvas buffer"),
                                size: (align_up(settings.width, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                                    * settings.height
                                    * 4) as u64,
                                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                                mapped_at_creation: false,
                            }));

                            Some(FemtovgInfo {
                                renderer: FemtovgRenderer::new(lts.clone()),
                                canvas,
                                texture,
                                buffer,
                            })
                        } else {
                            None
                        };
                    }
                    state.reload_lts.store(false, Ordering::Relaxed);
                }

                if let Some(viewer) = state.viewer.lock().unwrap().as_mut() {
                    let start = Instant::now();

                    match cli.viewer {
                        ViewerType::Cpu => {
                            // Resize the local pixel buffer if necessary
                            if pixel_buffer.width() != settings.width || pixel_buffer.height() != settings.height {
                                *pixel_buffer = SharedPixelBuffer::<Rgba8Pixel>::new(settings.width, settings.height);
                            }

                            let mut image = tiny_skia::PixmapMut::from_bytes(
                                pixel_buffer.make_mut_bytes(),
                                settings.width,
                                settings.height,
                            )
                            .unwrap();

                            if let Some(skia_renderer) = skia_renderer {
                                skia_renderer.render(
                                    &mut image,
                                    viewer,
                                    settings.draw_action_labels,
                                    settings.state_radius,
                                    settings.view_x,
                                    settings.view_y,
                                    settings.width,
                                    settings.height,
                                    settings.zoom_level,
                                    settings.label_text_size,
                                );
                            }

                            *state.canvas.lock().unwrap() = pixel_buffer.clone();
                        }
                        ViewerType::Gpu => {
                            let (device, queue) = wgpu.as_ref().expect("GPU rendering requires wgpu to be initialized");

                            // Render the graph using femtovg on the GPU
                            if let Some(femtovg_info) = femtovg_info {
                                if femtovg_info.texture.width() != settings.width
                                    || femtovg_info.texture.height() != settings.height
                                {
                                    // Create the texture and buffer for the femtovg renderer
                                    femtovg_info.texture = device.create_texture(&TextureDescriptor {
                                        label: Some("ltsgraph canvas texture"),
                                        size: wgpu::Extent3d {
                                            width: settings.width,
                                            height: settings.height,
                                            depth_or_array_layers: 1,
                                        },
                                        mip_level_count: 1,
                                        sample_count: 1,
                                        dimension: wgpu::TextureDimension::D2,
                                        format: TextureFormat::Rgba8UnormSrgb,
                                        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
                                        view_formats: &[],
                                    });

                                    // This buffer is used to copy the rendered image to the texture.
                                    femtovg_info.buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
                                        label: Some("ltsgraph canvas buffer"),
                                        size: (align_up(settings.width, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                                            * settings.height
                                            * 4) as u64,
                                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                                        mapped_at_creation: false,
                                    }));
                                }

                                // Render the texture using femtovg
                                femtovg_info.renderer.render(
                                    &mut femtovg_info.canvas,
                                    viewer,
                                    settings.draw_action_labels,
                                    settings.state_radius,
                                    settings.view_x,
                                    settings.view_y,
                                    settings.width,
                                    settings.height,
                                    settings.zoom_level,
                                    settings.label_text_size,
                                )?;

                                let buffer = femtovg_info.canvas.flush_to_surface(&femtovg_info.texture);

                                // Copy the texture to a buffer such that it can be mapped on the CPU
                                let copy = femtovg_info.texture.as_image_copy();

                                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                    label: Some("ltsgraph canvas encoder"),
                                });

                                encoder.copy_texture_to_buffer(
                                    copy,
                                    wgpu::TexelCopyBufferInfo {
                                        buffer: &femtovg_info.buffer,
                                        layout: wgpu::TexelCopyBufferLayout {
                                            offset: 0,
                                            bytes_per_row: Some(
                                                align_up(settings.width, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * 4,
                                            ),
                                            rows_per_image: Some(settings.height),
                                        },
                                    },
                                    wgpu::Extent3d {
                                        width: femtovg_info.texture.width(),
                                        height: femtovg_info.texture.height(),
                                        depth_or_array_layers: 1,
                                    },
                                );

                                queue.submit([buffer, encoder.finish()]);

                                let buffer = femtovg_info.buffer.clone();
                                let state = state.clone();
                                femtovg_info
                                    .buffer
                                    .slice(..)
                                    .map_async(wgpu::MapMode::Read, move |result| {
                                        if result.is_ok() {
                                            // Copy the data from the buffer to the pixel buffer

                                            let mut canvas = state.canvas.lock().unwrap();

                                            *canvas = SharedPixelBuffer::clone_from_slice(
                                                buffer.slice(..).get_mapped_range().deref(),
                                                settings.width,
                                                settings.height,
                                            );

                                            buffer.unmap();
                                        }
                                    });

                                // Wait for the buffer to be mapped and the data to be copied.
                                device.poll(wgpu::PollType::Wait {
                                    submission_index: None,
                                    timeout: None,
                                })?;
                            }
                        }
                    }

                    debug!(
                        "Rendering step ({} by {}) took {} ms",
                        settings.width,
                        settings.height,
                        (Instant::now() - start).as_millis()
                    );
                } else {
                    // If we are not rendering the graph, we still need to ensure the canvas is initialized.
                    let mut canvas = state.canvas.lock().unwrap();
                    if canvas.width() != settings.width || canvas.height() != settings.height {
                        *canvas = SharedPixelBuffer::<Rgba8Pixel>::new(settings.width, settings.height);
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

                if let Some(layout) = state.graph_layout.lock().unwrap().as_mut() {
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
                    if let Some(viewer) = state.viewer.lock().unwrap().as_mut() {
                        viewer.update(layout);
                    }

                    // Request a redraw (if not already in progress).
                    render_handle.resume();
                }

                // If stable pause the thread.
                Ok(!is_stable)
            },
        )?)
    };

    // Load an LTS from the given path and updates the state.
    let load_lts = {
        let state = state.clone();
        let layout_handle = layout_handle.clone();
        let render_handle = render_handle.clone();

        move |path: &Path, format: Option<LtsFormat>| -> Result<(), MercError> {
            debug!("Loading LTS {} ...", path.to_string_lossy());

            let format = guess_lts_format_from_extension(path, format).ok_or("Unknown LTS file format.")?;
            let mut timing = Timing::new();
            match read_explicit_lts(path, format, vec![], &mut timing) {
                Ok(lts) => {
                    // Ensure that the labels are strings, such that they can displayed.
                    let lts: Arc<LabelledTransitionSystem<String>> =
                        apply_lts!(lts, (), |lts, _| { Arc::new(lts.relabel(|label| label.to_string())) });

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

                    state.viewer.lock().unwrap().replace(viewer);
                    state.graph_layout.lock().unwrap().replace(layout);

                    // Indicate that the LTS has been loaded such that the rendering thread can be updated.
                    state.lts.lock().unwrap().replace(lts);
                    state.reload_lts.store(true, Ordering::Relaxed);

                    // Enable the layout and rendering threads.
                    layout_handle.resume();
                    render_handle.resume();
                    Ok(())
                }
                Err(x) => Ok(show_error_dialog("Failed to load LTS!", &format!("{x}"))),
            }
        }
    };

    // When the simulation is toggled enable the layout thread.
    {
        let layout_handle = layout_handle.clone();
        app.on_run_simulation(move |enabled| {
            if enabled {
                layout_handle.resume();
            } else {
                layout_handle.pause();
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
                    {
                        if load_lts(handle.path(), cli.lts_format).is_err() {
                            warn!("Failed to load LTS from file dialog.");
                        }
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
            if let Some(app) = app_weak.upgrade() {
                if let Some(viewer) = state.viewer.lock().unwrap().as_ref() {
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
            }
        });
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

    Ok(ExitCode::SUCCESS)
}
