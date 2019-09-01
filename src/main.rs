extern crate chrono;
extern crate clap;
extern crate regex;

use chrono::NaiveTime;
use clap::{Arg, App};
use regex::Regex;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;

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
    let delay = if let Some(delay_captures) = delay_regex.captures(opts.input_aud
                                                                       .to_str()
                                                                       .unwrap()) {
        delay_captures[1].parse::<isize>().unwrap()
    } else {
        0isize
    };

    // Read in the contents of the avisynth script
    let mut avs_file = File::open(&opts.input_avs).unwrap();
    let mut avs_contents = String::new();
    avs_file.read_to_string(&mut avs_contents).ok();

    // Determine where to trim
    // This is not the best regex--it takes ALL TRIMS and includes them
    let trim_regex = Regex::new(r"[tT]rim\((?:\w+, ?)?(\d+), ?(\d+)\)").unwrap();
    // A vector of timestamps for trimming
    let mut cut_times: Vec<String> = Vec::new();
    for capture_group in trim_regex.captures_iter(&avs_contents) {
        for (_, capture) in capture_group.iter().enumerate().filter(|&(i, _)| i % 3 != 0) {
            let frame: usize = capture.unwrap().as_str().parse().unwrap();
            let seconds: f32 = frame as f32 / opts.framerate;
            let nano: f32 = seconds.fract() * 1_000_000_000f32;
            let timestamp = NaiveTime::from_num_seconds_from_midnight(seconds.trunc() as u32,
                                                                      nano as u32);
            cut_times.push(timestamp.format("%H:%M:%S%.3f").to_string());
        }
    }

    if cut_times.is_empty() {
        panic!("No trims found in avs file");
    }

    // Split the audio file apart
    eprintln!("Splitting audio file with {} delay", delay);
    let output = Command::new("mkvmerge")
                     .arg("-o")
                     .arg(opts.output_aud.with_extension("split.mka").to_str().unwrap())
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
            merge_files.push(opts.output_aud.with_extension(format!("split-{:03}.mka", i + 1)));
        }
    }

    let output = Command::new("mkvmerge")
                     .arg("-o")
                     .arg(opts.output_aud.to_str().unwrap())
                     .args(&merge_files.iter()
                                       .enumerate()
                                       .map(|(i, x)| {
                                           if i == 0 {
                                               x.to_str().unwrap().to_owned()
                                           } else {
                                               format!("+{}", x.to_str().unwrap())
                                           }
                                       })
                                       .collect::<Vec<String>>())
                     .output()
                     .unwrap_or_else(|e| panic!("failed to execute process: {}", e));
    println!("{}", String::from_utf8(output.stdout).unwrap());
}

fn main() {
    let matches =
        App::new("split_aud")
            .version("0.1")
            .arg(Arg::with_name("framerate")
                     .short("f")
                     .long("framerate")
                     .value_name("FRACTION")
                     .help("Set a custom framerate (default 30000/1001)")
                     .takes_value(true))
            .arg(Arg::with_name("input")
                     .short("i")
                     .long("input")
                     .help("Sets the input aac file to use")
                     .takes_value(true)
                     .required(true))
            .arg(Arg::with_name("output")
                     .short("o")
                     .long("output")
                     .help("Sets the output mka file to write to (default: avs path plus .mka)")
                     .takes_value(true))
            .arg(Arg::with_name("avs")
                     .help("Sets the input avs file to use")
                     .required(true)
                     .takes_value(true)
                     .index(1))
            .arg(Arg::with_name("verbosity")
                     .short("v")
                     .long("verbose")
                     .help("Sets the level of verbosity"))
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
