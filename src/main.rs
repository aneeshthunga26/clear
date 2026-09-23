use clear::runtime::Options;
use std::{path::PathBuf, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    let Some(options) = parse_args(std::env::args().skip(1))? else {
        return Ok(());
    };
    clear::platform::run(options)
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut args = args;
    let mut options = Options::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "Clear — nested Wayland compositor\n\nUsage: clear [OPTIONS] [--command PROGRAM [ARG...]]\n\n  --config PATH        Load a TOML config instead of the XDG default\n  --socket NAME        Use a specific Wayland socket name\n  --exit-after SECONDS  Gracefully stop after a bounded test run\n  --capture PATH       Save a PPM frame near the test deadline (or after 3s)\n  --command, -c        Launch a child inside Clear (must be the last option)\n  --help, -h           Show this help\n\nDefaults: two virtual monitors, nine workspaces. Super+Return opens foot;\nSuper+Escape exits. See examples/config.toml for bindings and Rhai extensions."
                );
                return Ok(None);
            }
            "--config" => {
                options.config_path = Some(PathBuf::from(
                    args.next().ok_or("--config requires a path")?,
                ))
            }
            "--capture" => {
                options.capture = Some(PathBuf::from(
                    args.next().ok_or("--capture requires a path")?,
                ))
            }
            "--socket" => {
                let name = args.next().ok_or("--socket requires a name")?;
                if name.is_empty() || name.contains('/') || name == "." || name == ".." {
                    return Err("socket must be a nonempty filename, not a path".into());
                }
                options.socket_name = Some(name);
            }
            "--exit-after" => {
                let seconds: u64 = args
                    .next()
                    .ok_or("--exit-after requires seconds")?
                    .parse()
                    .map_err(|_| "--exit-after must be a positive integer")?;
                if !(1..=86400).contains(&seconds) {
                    return Err("--exit-after must be in 1..=86400".into());
                }
                options.exit_after = Some(Duration::from_secs(seconds));
            }
            "--command" | "-c" => {
                options.command = args.collect();
                if options.command.is_empty() || options.command[0].is_empty() {
                    return Err("--command requires an executable".into());
                }
                return Ok(Some(options));
            }
            _ => return Err(format!("unknown option {arg:?}; use --help")),
        }
    }
    Ok(Some(options))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<Option<Options>, String> {
        parse_args(args.iter().map(|a| a.to_string()))
    }
    #[test]
    fn bounded_runs_and_child_arguments() {
        let options = parse(&[
            "--exit-after",
            "3",
            "--socket",
            "clear-test",
            "--command",
            "foot",
            "--app-id",
            "demo",
        ])
        .unwrap()
        .unwrap();
        assert_eq!(options.exit_after, Some(Duration::from_secs(3)));
        assert_eq!(options.command, ["foot", "--app-id", "demo"]);
    }
    #[test]
    fn rejects_invalid_arguments() {
        for args in [
            vec!["--socket", "../bad"],
            vec!["--exit-after", "0"],
            vec!["--exit-after", "NaN"],
            vec!["--command"],
            vec!["--config"],
            vec!["--unknown"],
        ] {
            assert!(parse(&args).is_err());
        }
    }
}
