use std::collections::BTreeMap;
use std::path::Path;

use crate::managed_agents::{
    discovery::{classify_runtime, codex_adapter_availability, find_command, KnownAcpRuntime},
    AcpAvailabilityStatus,
};

use super::{cli_probe, Requirement};

/// Codex requirements, probed with the engine this agent's codex-acp will run
/// (`CODEX_PATH` from `agent_env` or the app env, else the bundled engine).
pub(super) fn codex_requirements(
    runtime: &KnownAcpRuntime,
    agent_env: &BTreeMap<String, String>,
) -> Vec<Requirement> {
    let codex_path = cli_probe::codex_path_env(Some(agent_env));
    probe_requirements(
        &["codex", "login", "status"],
        "run `codex login`",
        runtime,
        codex_path.as_deref(),
    )
}

/// Requirements for a CLI-login runtime probed with `probe_args` as given
/// (claude; codex goes through [`codex_requirements`]).
pub(super) fn requirements(
    probe_args: &[&str],
    setup_copy: &str,
    runtime: &KnownAcpRuntime,
) -> Vec<Requirement> {
    probe_requirements(probe_args, setup_copy, runtime, None)
}

fn probe_requirements(
    probe_args: &[&str],
    setup_copy: &str,
    runtime: &KnownAcpRuntime,
    codex_path: Option<&str>,
) -> Vec<Requirement> {
    let adapter_result = runtime
        .commands
        .iter()
        .find_map(|cmd| find_command(cmd).map(|path| (*cmd, path)));
    let underlying_cli_found = runtime
        .underlying_cli
        .map(|cli| find_command(cli).is_some())
        .unwrap_or(false);

    let (availability, _cmd, adapter_path) =
        classify_runtime(adapter_result, runtime.underlying_cli, underlying_cli_found);
    let availability = if runtime.id == "codex" && availability == AcpAvailabilityStatus::Available
    {
        adapter_path
            .as_deref()
            .map(|path| codex_adapter_availability(Path::new(path)))
            .unwrap_or(availability)
    } else {
        availability
    };

    match availability {
        AcpAvailabilityStatus::Available => {
            let adapter_path = adapter_path.as_deref().map(Path::new);
            let Some((program, argv)) =
                cli_probe::probe_command(probe_args, adapter_path, codex_path)
            else {
                return vec![missing_requirement(
                    probe_args,
                    setup_copy,
                    AcpAvailabilityStatus::Available,
                )];
            };
            let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
            let augmented_path = cli_probe::augmented_path();
            match cli_probe::login_probe(&program, &argv, augmented_path.as_deref()) {
                cli_probe::ProbeOutcome::LoggedIn => vec![],
                cli_probe::ProbeOutcome::LoggedOut => vec![missing_requirement(
                    probe_args,
                    setup_copy,
                    AcpAvailabilityStatus::Available,
                )],
                cli_probe::ProbeOutcome::ConfigInvalid { stderr_excerpt } => {
                    vec![Requirement::CliConfigInvalid {
                        probe_args: probe_args.iter().map(|value| value.to_string()).collect(),
                        setup_copy: setup_copy.to_string(),
                        diagnostic: stderr_excerpt,
                    }]
                }
            }
        }
        other => vec![missing_requirement(probe_args, setup_copy, other)],
    }
}

fn missing_requirement(
    probe_args: &[&str],
    setup_copy: &str,
    availability: AcpAvailabilityStatus,
) -> Requirement {
    Requirement::CliLogin {
        probe_args: probe_args.iter().map(|value| value.to_string()).collect(),
        setup_copy: setup_copy.to_string(),
        availability,
    }
}
