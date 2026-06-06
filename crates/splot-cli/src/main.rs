// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <contact@splotlabs.io>

//! `splot` — a thin command-line interface over the `splot-*` library crates.
//!
//! This binary only parses arguments, initializes logging, reads/writes files,
//! and calls into the libraries. All codec and validation logic lives in the
//! library crates.

use std::process::ExitCode;

use clap::{Parser, Subcommand};

mod commands;

/// splot — an AV2 bitstream validator/inspector and future encoder.
#[derive(Parser, Debug)]
#[command(
    name = "splot",
    version,
    about = "splot — an AV2 bitstream validator/inspector and future encoder",
    propagate_version = true,
    after_help = "splot is free for noncommercial use under PolyForm Noncommercial 1.0.0.\nCommercial use of ANY component (validator, inspector, encoder, CLI) requires a\nseparate commercial license: contact@splotlabs.io."
)]
struct Cli {
    /// Increase logging verbosity (repeatable: -v, -vv, -vvv).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
    /// Silence all logging except errors.
    #[arg(long, global = true)]
    quiet: bool,
    #[command(subcommand)]
    command: Command,
}

/// Top-level subcommands.
#[derive(Subcommand, Debug)]
enum Command {
    /// Validate an AV2 length-delimited bitstream.
    #[command(visible_aliases = ["val", "check"])]
    Validate(commands::validate::ValidateArgs),
    /// Print OBUs and headers from a bitstream.
    #[command(visible_alias = "dump")]
    Inspect(commands::inspect::InspectArgs),
    /// Future AV2 encoder entry point.
    #[command(visible_alias = "enc")]
    Encode(commands::encode::EncodeArgs),
    /// Future reference-style decode / round-trip test entry point.
    #[command(visible_alias = "dec")]
    Decode(commands::decode::DecodeArgs),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    commands::init_tracing(cli.verbose, cli.quiet);

    let result = match cli.command {
        Command::Validate(args) => commands::validate::run(&args),
        Command::Inspect(args) => commands::inspect::run(&args),
        Command::Encode(args) => commands::encode::run(&args),
        Command::Decode(args) => commands::decode::run(&args),
    };

    match result {
        Ok(code) => code,
        Err(error) => {
            tracing::error!("{error:#}");
            eprintln!("error: {error:#}");
            ExitCode::from(2)
        }
    }
}
