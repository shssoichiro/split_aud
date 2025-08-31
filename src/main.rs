#![warn(clippy::all)]

use chrono::NaiveTime;
use clap::{App, Arg};
use regex::Regex;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

fn get_total_frames(input_path: &Path, framerate: f32) -> isize {
    let out = match Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(input_path.to_str().unwrap())
        .output()
    {
        Ok(o) => o,
        Err(_) => return 0,
    };
    if !out.status.success() {
        return 0;
    }
    let output = String::from_utf8_lossy(&out.stdout);
    let seconds: f64 = output
        .trim()
        .parse()
        .expect("failed to parse audio duration");
    let total_frames = (seconds * framerate as f64).floor() as isize;
    if total_frames < 0 { 0 } else { total_frames }
}

#[derive(Debug, Clone)]
struct Config {
    framerate: f32,
    input_aud: PathBuf,
    output_aud: PathBuf,
    input_avs: PathBuf,
    verbose: bool,
}

fn split_audio(opts: &Config) {
    // Determine if we should apply a delay to the audio
    let delay_regex = Regex::new(r"DELAY (-?\d+)ms").unwrap();
    let delay = if let Some(delay_captures) = delay_regex.captures(opts.input_aud.to_str().unwrap())
    {
        delay_captures[1].parse::<isize>().unwrap()
    } else {
        0isize
    };

    // Read in the contents of the avisynth script
    let mut avs_file = File::open(&opts.input_avs).unwrap();
    let mut avs_contents = String::new();
    avs_file.read_to_string(&mut avs_contents).ok();

    // Determine where to trim
    // A vector of timestamps for trimming
    let mut cut_times: Vec<String> = Vec::new();
    // This is not the best regex--it takes ALL TRIMS and includes them
    let trim_regex = Regex::new(r"[tT]rim\((?:\w+, ?)?(\d+), ?(\d+)\)").unwrap();
    for capture_group in trim_regex.captures_iter(&avs_contents) {
        for (_, capture) in capture_group
            .iter()
            .enumerate()
            .filter(|&(i, _)| i % 3 != 0)
        {
            let frame: usize = capture.unwrap().as_str().parse().unwrap();
            let seconds: f32 = frame as f32 / opts.framerate;
            let nano: f32 = seconds.fract() * 1_000_000_000f32;
            let timestamp =
                NaiveTime::from_num_seconds_from_midnight_opt(seconds.trunc() as u32, nano as u32)
                    .unwrap();
            cut_times.push(timestamp.format("%H:%M:%S%.3f").to_string());
        }
    }

    if cut_times.is_empty() {
        // And for supporting python slice syntax
        let trim_regex = Regex::new(r"clip\[(\d+): ?(-?\d+)\]").unwrap();
        let mut cached_total_frames: Option<isize> = None;
        for capture_group in trim_regex.captures_iter(&avs_contents) {
            for (i, capture) in capture_group
                .iter()
                .enumerate()
                .filter(|&(i, _)| i % 3 != 0)
            {
                let value_str = capture.unwrap().as_str();
                let frame_isize: isize = if i == 2 {
                    let end_index = value_str.parse::<isize>().unwrap();
                    if end_index < 0 {
                        // Support negative end indices which count from the end.
                        let total_frames = *cached_total_frames.get_or_insert_with(|| {
                            get_total_frames(&opts.input_aud, opts.framerate)
                        });
                        total_frames + end_index
                    } else {
                        // For python slice syntax, positive end index is exclusive; adjust by -1.
                        end_index - 1
                    }
                } else {
                    value_str.parse::<isize>().unwrap()
                };
                let frame: usize = if frame_isize < 0 {
                    0
                } else {
                    frame_isize as usize
                };
                let seconds: f32 = frame as f32 / opts.framerate;
                let nano: f32 = seconds.fract() * 1_000_000_000f32;
                let timestamp = NaiveTime::from_num_seconds_from_midnight_opt(
                    seconds.trunc() as u32,
                    nano as u32,
                )
                .unwrap();
                cut_times.push(timestamp.format("%H:%M:%S%.3f").to_string());
            }
        }
    }

    if cut_times.is_empty() {
        panic!("No trims found in avs file");
    }

    // Split the audio file apart
    eprintln!("Splitting audio file with {} delay", delay);
    let output = Command::new("mkvmerge")
        .arg("-o")
        .arg(
            opts.output_aud
                .with_extension("split.mka")
                .to_str()
                .unwrap(),
        )
        .arg("--sync")
        .arg(format!("0:{}", delay))
        .arg(opts.input_aud.to_str().unwrap())
        .arg("--split")
        .arg(format!("timecodes:{}", cut_times.join(",")))
        .output()
        .unwrap_or_else(|e| panic!("failed to execute process: {}", e));
    println!("{}", String::from_utf8(output.stdout).unwrap());

    // Put it back together
    let mut merge_files: Vec<PathBuf> = Vec::new();
    let mut use_first = false;
    for (i, timestamp) in cut_times.iter().enumerate() {
        if i == cut_times.len() && use_first {
            break;
        }
        if i == 0 && timestamp == "00:00:00.000" {
            use_first = true;
        }
        if (use_first && i % 2 == 0) || (!use_first && i % 2 == 1) {
            merge_files.push(
                opts.output_aud
                    .with_extension(format!("split-{:03}.mka", i + 1)),
            );
        }
    }

    let output = Command::new("mkvmerge")
        .arg("-o")
        .arg(opts.output_aud.to_str().unwrap())
        .args(
            merge_files
                .iter()
                .enumerate()
                .map(|(i, x)| {
                    if i == 0 {
                        x.to_str().unwrap().to_owned()
                    } else {
                        format!("+{}", x.to_str().unwrap())
                    }
                })
                .collect::<Vec<String>>(),
        )
        .output()
        .unwrap_or_else(|e| panic!("failed to execute process: {}", e));
    println!("{}", String::from_utf8(output.stdout).unwrap());

    println!("Cleaning temporary files...");
    let split_regex = Regex::new(r"split-(?:\d{3})\.mka$").unwrap();
    let dir = opts.output_aud.parent().unwrap();
    for file in dir
        .read_dir()
        .unwrap()
        .flatten()
        .filter(|file| split_regex.is_match(&file.file_name().to_string_lossy()))
    {
        let _ = std::fs::remove_file(file.path());
    }
}

fn main() {
    let matches = App::new("split_aud")
        .version("0.1")
        .arg(
            Arg::with_name("framerate")
                .short("f")
                .long("framerate")
                .value_name("FRACTION")
                .help("Set a custom framerate (default 30000/1001)")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("input")
                .short("i")
                .long("input")
                .help("Sets the input audio file to use")
                .takes_value(true)
                .required(true),
        )
        .arg(
            Arg::with_name("output")
                .short("o")
                .long("output")
                .help("Sets the output mka file to write to (default: avs path plus .mka)")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("avs")
                .help("Sets the input avs or vpy file to use")
                .required(true)
                .takes_value(true)
                .index(1),
        )
        .arg(
            Arg::with_name("verbosity")
                .short("v")
                .long("verbose")
                .help("Sets the level of verbosity"),
        )
        .get_matches();

    let mut options = Config {
        framerate: 30000f32 / 1001f32,
        input_aud: PathBuf::new(),
        output_aud: PathBuf::new(),
        input_avs: PathBuf::new(),
        verbose: false,
    };

    if matches.is_present("framerate") {
        let parts: Vec<&str> = matches.value_of("framerate").unwrap().split('/').collect();
        let framerate_num = parts[0].parse::<f32>().unwrap();
        let framerate_den = parts[1].parse::<f32>().unwrap();
        options.framerate = framerate_num / framerate_den;
    }

    options.input_aud = PathBuf::from(matches.value_of("input").unwrap());
    options.input_avs = PathBuf::from(matches.value_of("avs").unwrap());
    if matches.is_present("output") {
        options.output_aud = PathBuf::from(matches.value_of("output").unwrap());
    } else {
        options.output_aud = options.input_avs.with_extension("mka");
    }

    options.verbose = matches.is_present("verbosity");

    split_audio(&options);
}
