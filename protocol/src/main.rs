//! `airp-protocol` — a tiny CLI that validates AIRP State Protocol envelopes.
//!
//! Reads JSON envelopes from the given files (or stdin if none) and checks each
//! parses as a valid [`Envelope`]. Exits non-zero if any fail. Useful for CI,
//! editor hooks, or sanity-checking hand-written messages.

use std::io::Read;
use std::process::ExitCode;

use airp_state_protocol::{Body, Envelope};

const HELP: &str = "\
airp-protocol — validate AIRP State Protocol envelopes

USAGE:
    airp-protocol [FILE ...]
    cat envelope.json | airp-protocol

Reads JSON envelopes from the given files (or stdin if none are given) and
checks each one parses as a valid protocol Envelope. Exit code is 1 if any
input fails to validate.";

fn kind_of(body: &Body) -> &'static str {
    match body {
        Body::Blueprint(_) => "blueprint",
        Body::State(_) => "state",
        Body::Manifest(_) => "manifest",
        Body::Event(_) => "event",
        Body::Error(_) => "error",
        Body::Intent(_) => "intent",
        Body::Subscribe(_) => "subscribe",
        Body::Unsubscribe(_) => "unsubscribe",
        Body::Hello(_) => "hello",
        Body::Ack(_) => "ack",
    }
}

fn validate(label: &str, text: &str) -> bool {
    match serde_json::from_str::<Envelope>(text) {
        Ok(env) => {
            println!("OK   {label}: v{} kind={} src={}", env.v, kind_of(&env.body), env.src);
            true
        }
        Err(e) => {
            eprintln!("FAIL {label}: {e}");
            false
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }

    let mut ok = true;

    if args.is_empty() {
        let mut buf = String::new();
        if std::io::stdin().read_to_string(&mut buf).is_err() {
            eprintln!("FAIL <stdin>: could not read stdin");
            return ExitCode::FAILURE;
        }
        ok &= validate("<stdin>", &buf);
    } else {
        for path in &args {
            match std::fs::read_to_string(path) {
                Ok(text) => ok &= validate(path, &text),
                Err(e) => {
                    eprintln!("FAIL {path}: {e}");
                    ok = false;
                }
            }
        }
    }

    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
