//! Command-line actions that run without opening the editor window.
//!
//! ```text
//! evolve_editor --sample <out.evolve>                   write the demo project
//! evolve_editor --export-desktop <in.evolve> <out>      build a standalone game (<out>.exe on Windows, <out>.app on macOS)
//! evolve_editor --export-web <in.evolve> <out-folder>   build a browser game
//! evolve_editor --export-png <in.evolve> <out.png> [scale]  render what the camera of the start scene sees
//! evolve_editor --export-svg <in.evolve> <out.svg>      write the start scene as SVG
//! evolve_editor --gen-scene <kind> <out.evolve>         write a benchmark scene that needs no game files (kinds: large, illustration, sprites)
//! evolve_editor --benchmark <in.evolve> <report> [--allow-long] [--gpu-draw-supported]
//!                                                       measure rendering (CPU and, with a graphics card, GPU) and editing
//!                                                       cost; writes <report>.json and <report>.txt; stops after 5 minutes
//!                                                       unless --allow-long is given; --gpu-draw-supported times the GPU
//!                                                       renderer on scenes it cannot draw completely yet (timing only)
//! ```

use crate::sample;
use evolve_engine::export_image;
use evolve_engine::io;
use evolve_player_core::bundle;
use std::path::Path;

/// Runs a command-line action if one was requested. Returns false to start the editor normally.
pub fn run(args: &[String]) -> bool {
    let result = match args {
        [flag, out] if flag == "--sample" => io::save_project(&sample::project(), Path::new(out)).map(|()| format!("wrote {out}")),
        [flag, project, out] if flag == "--export-desktop" => export_desktop(project, out),
        [flag, project, out] if flag == "--export-web" => export_web(project, out),
        [flag, kind, out] if flag == "--gen-scene" => gen_scene(kind, out),
        [flag, project, report, rest @ ..] if flag == "--benchmark" && rest.iter().all(|a| a == "--allow-long" || a == "--gpu-draw-supported") => {
            let options = crate::bench::BenchOptions {
                allow_long: rest.iter().any(|a| a == "--allow-long"),
                gpu_draw_supported: rest.iter().any(|a| a == "--gpu-draw-supported"),
            };
            crate::bench::run(project, report, options)
        }
        [flag, project, out] if flag == "--export-svg" => export_svg(project, out),
        [flag, project, out, rest @ ..] if flag == "--export-png" => export_png(project, out, rest.first().and_then(|s| s.parse().ok()).unwrap_or(1.0)),
        _ => return false,
    };
    match result {
        Ok(message) => println!("{message}"),
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
    }
    true
}

fn gen_scene(kind: &str, out: &str) -> io::Result<String> {
    let project = crate::bench_scene::generate(kind).ok_or_else(|| format!("unknown scene kind \"{kind}\"; known kinds: {}", crate::bench_scene::KINDS.join(", ")))?;
    io::save_project(&project, Path::new(out))?;
    Ok(format!("wrote {out}"))
}

fn export_svg(project: &str, out: &str) -> io::Result<String> {
    let project = io::load_project(Path::new(project))?;
    std::fs::write(out, export_image::to_svg(&project.scenes[project.start]))?;
    Ok(format!("exported {out}"))
}

fn export_png(project: &str, out: &str, scale: f32) -> io::Result<String> {
    let project = io::load_project(Path::new(project))?;
    let pixmap = export_image::render_artboard(&project.scenes[project.start], scale, false).ok_or("the picture is too large to render")?;
    pixmap.save_png(out)?;
    Ok(format!("exported {out} ({} x {})", pixmap.width(), pixmap.height()))
}

fn export_desktop(project: &str, out: &str) -> io::Result<String> {
    let player = bundle::find_player_exe().ok_or("evolve_player is missing next to the editor (cargo build -p evolve_player)")?;
    let json = io::project_to_string(&io::load_project(Path::new(project))?)?;
    bundle::export_desktop(&player, &json, Path::new(out))?;
    Ok(format!("exported {out}"))
}

fn export_web(project: &str, out: &str) -> io::Result<String> {
    let player_dir = bundle::find_web_player_dir().ok_or("the web player is not built (tools/build-web-player.ps1 or .sh)")?;
    let json = io::project_to_string(&io::load_project(Path::new(project))?)?;
    bundle::export_web(&player_dir, &json, Path::new(out))?;
    Ok(format!("exported {out}"))
}
