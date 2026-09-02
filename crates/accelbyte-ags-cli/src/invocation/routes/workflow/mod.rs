//! `ags workflow run <id>` — execute a registered workflow by id.
//!
//! Illegal-flag rejections (`--skeleton`), `--help`, the
//! registry lookup, and the workflow's own `--<input>` flag parse all
//! happen before the runtime prologue, so a bad invocation reports a usage
//! error instead of failing late on auth / base-URL resolution.

use ags_protocol::workflow::WorkflowId;
use ags_runtime::runtime::workflows::compile::compile_workflow;
use ags_runtime::runtime::workflows::{registry, RunOptions};

use crate::errors::CliError;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::phase_execution::{
    flush_step_telemetry, run_phase_owned_execution, AdapterMode,
};
use crate::invocation::shape::{RouteKind, Shape};
use crate::invocation::workflows::{build_registered_workflow_clap, coerce_cli_value};
use crate::invocation::InvocationOutcome;

/// Build the "unknown workflow id" usage error, mirroring the `Unknown
/// service: '…'` shape from the service route: a capitalised headline with the
/// offending id quoted, followed by the list of registered ids the user can
/// pick from.
/// Render a clap command's long help to stderr (UI chrome), styled with ANSI
/// when stderr supports colour — so `ags workflow` help matches the bold/under-
/// lined headings and bold commands clap prints for the rest of the CLI.
/// `StyledStr::to_string()` strips styling, so emit the ANSI form when colour
/// is enabled.
fn write_command_help(command: &mut clap::Command) {
    let help = command.render_long_help();
    let rendered = if crate::frontend::style::is_stderr_enabled() {
        help.ansi().to_string()
    } else {
        help.to_string()
    };
    let _ = crate::frontend::streams::UiSink.write_all(rendered.as_bytes());
}

/// Assemble the per-run step-telemetry sink from an already-resolved
/// identity and client. Split out from `route_workflow_run` so this mapping
/// is unit-testable on its own.
fn build_workflow_step_telemetry(
    sub: String,
    client: ags_runtime::runtime::telemetry::TelemetryClient,
    run_id: &str,
    workflow_id: &str,
    steps_total: usize,
    is_dry_run: bool,
    ui_surface: &'static str,
) -> Box<crate::frontend::sink::WorkflowStepTelemetry> {
    Box::new(crate::frontend::sink::WorkflowStepTelemetry {
        client,
        sub,
        context: ags_runtime::runtime::telemetry::WorkflowStepContext {
            run_id: run_id.to_string(),
            workflow_id: workflow_id.to_string(),
            steps_total,
            cli_version: env!("CARGO_PKG_VERSION").to_string(),
            is_dry_run,
            ui_surface,
        },
    })
}

/// Build the protocol-version-mismatch pre-run warning for
/// `ags workflow run`. Pure string construction, no I/O — the caller writes
/// it to stderr. A provenance hint, not a diagnosis: states both versions as
/// plain facts, never claims the workflow *is* broken, and never frames
/// either version as a hard compatibility limit (no "understands"/"supports"
/// wording). Fires for a mismatch in either direction — older or newer than
/// this CLI build's own protocol version — with the same neutral phrasing.
fn mismatched_protocol_version_warning(workflow_id: &str, declared: &str, current: &str) -> String {
    format!(
        "Workflow '{workflow_id}' targets protocol version {declared}; this ags build is on \
         protocol version {current}. If you encounter issues, this may be why."
    )
}

/// Same provenance-hint role as `mismatched_protocol_version_warning`, for the
/// legacy case: a `workflow_protocol_version`-less file, registered as such by
/// `external::load_external_workflows` (installed before `ags workflow add`
/// required the field). There is no declared version to compare, so this
/// states that fact instead of a specific mismatch.
fn legacy_protocol_version_warning(workflow_id: &str, current: &str) -> String {
    format!(
        "Workflow '{workflow_id}' does not declare a protocol version (likely installed before \
         this CLI started requiring one); this ags build is on protocol version {current}. If \
         you encounter issues, this may be why."
    )
}

/// Same provenance-hint role as the sibling warning functions, for the case
/// where a workflow declares a `workflow_protocol_version` that does not parse
/// as a version number at all. Quotes the actual declared value so the user
/// can locate and correct it in the file.
fn unreadable_protocol_version_warning(workflow_id: &str, declared: &str, current: &str) -> String {
    format!(
        "Workflow '{workflow_id}' declares a protocol version that is not a readable version \
         number: \"{declared}\". This ags build is on protocol version {current}. The field can \
         be corrected by hand or by re-adding the workflow with `ags workflow add`."
    )
}

fn unknown_workflow_error(workflow_id: &str) -> CliError {
    let workflows = registry()
        .ids()
        .map(|id| format!("  {}", id.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    CliError::Usage {
        message: format!("Unknown workflow: '{workflow_id}'\n\nValid workflows:\n{workflows}"),
        metadata: None,
    }
}

/// True when `remaining` (post-global-flag-prescan args) names `workflow run`.
///
/// Shared by the top-level [`crate::invocation::run`] special-case and any
/// future router check, so both agree on what counts as a workflow run.
pub(crate) fn is_workflow_run(remaining: &[String]) -> bool {
    remaining.first().map(String::as_str) == Some("workflow")
        && remaining.get(1).map(String::as_str) == Some("run")
}

/// Handle `ags workflow <subcommand>` for every subcommand except `run`.
///
/// `workflow run` is special-cased at the top level (it owns its own
/// presentation surfaces), so it never reaches here.
pub(crate) async fn route_workflow(
    args: &[String],
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    match args.first().map(String::as_str) {
        Some("run") => {
            // `workflow run` is routed directly from `invocation::run`, which
            // constructs its own surfaces after the prologue. Reaching here
            // means the top-level special-case was bypassed.
            unreachable!("workflow run is special-cased at the top level")
        }
        Some("list") => {
            // `list --help` must show the subcommand's help, not run the list.
            if args.iter().skip(1).any(|a| a == "--help" || a == "-h") {
                let command = crate::invocation::builder::build_workflow_command();
                if let Some(list) = command.find_subcommand("list") {
                    let mut list = list.clone().bin_name("ags workflow list");
                    write_command_help(&mut list);
                }
                Ok(InvocationOutcome::Complete)
            } else {
                route_workflow_list(frontend)
            }
        }
        Some("add") => {
            let mut command = crate::invocation::builder::build_workflow_command();
            let argv = crate::invocation::clap_helpers::build_argv("workflow", args);
            match command.try_get_matches_from_mut(argv.iter().map(String::as_str)) {
                Ok(matches) => {
                    let Some(("add", sub)) = matches.subcommand() else {
                        unreachable!("route_workflow only reaches this arm for 'add'");
                    };
                    let path = std::path::PathBuf::from(sub.get_one::<String>("path").unwrap());
                    let validate_only = sub.get_flag("validate-only");
                    route_workflow_add(&path, validate_only, frontend)
                }
                Err(error) => crate::invocation::clap_helpers::outcome_from_clap_error(error),
            }
        }
        Some("template") => {
            let mut command = crate::invocation::builder::build_workflow_command();
            let argv = crate::invocation::clap_helpers::build_argv("workflow", args);
            match command.try_get_matches_from_mut(argv.iter().map(String::as_str)) {
                Ok(matches) => {
                    let Some(("template", sub)) = matches.subcommand() else {
                        unreachable!("route_workflow only reaches this arm for 'template'");
                    };
                    let output_path = sub
                        .get_one::<String>("output")
                        .map(std::path::PathBuf::from);
                    route_workflow_template(output_path.as_deref(), frontend)
                }
                Err(error) => crate::invocation::clap_helpers::outcome_from_clap_error(error),
            }
        }
        Some("remove") => {
            let mut command = crate::invocation::builder::build_workflow_command();
            let argv = crate::invocation::clap_helpers::build_argv("workflow", args);
            match command.try_get_matches_from_mut(argv.iter().map(String::as_str)) {
                Ok(matches) => {
                    let Some(("remove", sub)) = matches.subcommand() else {
                        unreachable!("route_workflow only reaches this arm for 'remove'");
                    };
                    let id = sub.get_one::<String>("id").unwrap();
                    route_workflow_remove(id, frontend)
                }
                Err(error) => crate::invocation::clap_helpers::outcome_from_clap_error(error),
            }
        }
        Some("--help") | Some("-h") | None => {
            // Help text is UI chrome — route to stderr via UiSink
            // so stdout stays reserved for CommandOutput.
            let mut command =
                crate::invocation::builder::build_workflow_command().bin_name("ags workflow");
            write_command_help(&mut command);
            Ok(InvocationOutcome::Complete)
        }
        Some(other) => Err(CliError::Usage {
            message: format!("Unknown workflow subcommand: '{other}'"),
            metadata: None,
        }),
    }
}

/// Run a registered workflow: resolve its id, compile it, parse the
/// per-input flags, and drive it through the executor.
///
/// `workflow run` owns its presentation lifecycle. Pre-prologue work runs with
/// no surfaces and returns `Err(CliError)` for the caller to render. Workflow
/// surfaces are constructed only after `ExecutionContext::resolve` succeeds, so
/// a `--ui=fullscreen workflow run` that fails auth never grabs the terminal. Once
/// surfaces exist, execution failures are rendered on the owned frontend and
/// returned as `Ok` so the caller does not double-render.
pub(crate) async fn route_workflow_run(
    args: &[String],
    flags: &GlobalFlags,
    render_options: crate::frontend::RenderOptions,
    frontend_context: &crate::invocation::context::FrontendContext,
    workflow_run_id: Option<&str>,
) -> Result<InvocationOutcome, CliError> {
    // Illegal flags are rejected before any registry lookup so they fire
    // even against ids that do not exist.
    if flags.is_skeleton {
        return Err(CliError::Usage {
            message: "--skeleton is not supported with 'ags workflow run'".to_string(),
            metadata: None,
        });
    }

    // The id is the first non-flag token. Per the documented usage
    // (`ags workflow run <workflow-id> [OPTIONS]`) the id comes before any
    // `--<input>` flags; `--help`/`-h` may appear on either side of it.
    let id_index = args.iter().position(|a| !a.starts_with('-'));

    // `--help` is handled before the runtime prologue so it never triggers
    // auth resolution and never renders as a usage error.
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return render_workflow_run_help(id_index.map(|i| args[i].as_str()));
    }

    let id_index = id_index.ok_or_else(|| CliError::Usage {
        message: "Missing workflow id: ags workflow run <workflow-id>".to_string(),
        metadata: None,
    })?;
    let workflow_id = args[id_index].clone();
    let workflow_id_key = WorkflowId::new(workflow_id.as_str());

    let definition = match registry().resolve(&workflow_id_key) {
        Some(workflow) => workflow.definition().clone(),
        None => return Err(unknown_workflow_error(&workflow_id)),
    };
    // Resolved once here, before compilation, and threaded through
    // `RunOptions` — see §4.1 of the telemetry observability design: a
    // failed step's `input_fields` may carry real values only for a
    // bundled workflow.
    let workflow_is_bundled = registry().is_bundled(&workflow_id_key);

    // Informational only: computed here (never affects the return value or
    // exit code), but not written yet — see `execute_compiled_workflow` for
    // where and how it's actually emitted (gated on automation, deferred
    // past fullscreen teardown).
    let declared_protocol_version = definition.workflow_protocol_version.as_deref();
    let outdated_warning = if workflow_is_bundled || frontend_context.is_automation() {
        None
    } else {
        match declared_protocol_version {
            Some(declared)
                if ags_runtime::runtime::workflows::version_check::is_mismatched(Some(
                    declared,
                )) =>
            {
                Some(mismatched_protocol_version_warning(
                    &workflow_id,
                    declared,
                    ags_protocol::workflow::WORKFLOW_PROTOCOL_VERSION,
                ))
            }
            Some(declared)
                if ags_runtime::runtime::workflows::version_check::is_unparsable(declared) =>
            {
                Some(unreadable_protocol_version_warning(
                    &workflow_id,
                    declared,
                    ags_protocol::workflow::WORKFLOW_PROTOCOL_VERSION,
                ))
            }
            Some(_) => None,
            None => Some(legacy_protocol_version_warning(
                &workflow_id,
                ags_protocol::workflow::WORKFLOW_PROTOCOL_VERSION,
            )),
        }
    };

    // Spawned now, not where it's consumed below, so identity/client
    // resolution overlaps with compiling the workflow and the auth/base-URL
    // prologue instead of adding to this run's startup latency.
    let telemetry_task = workflow_run_id.map(|run_id| {
        let run_id = run_id.to_string();
        let profile = flags.profile.clone();
        tokio::spawn(async move {
            match ags_runtime::runtime::telemetry::resolve_identity(profile.as_deref()).await {
                Some(identity) => Some((
                    run_id,
                    identity.distinct_id().to_string(),
                    ags_runtime::runtime::telemetry::TelemetryClient::from_env().await,
                )),
                None => None,
            }
        })
    });

    // Compile the workflow and parse its own `--<input>` flags BEFORE the
    // runtime prologue, so an unknown flag surfaces as a clap usage error
    // rather than failing late on auth / base-URL resolution. Compilation
    // uses a bare catalogue; the executor consumes the compiled form as
    // data and does not recompile.
    let mut catalogue = ags_runtime::catalogue::Catalogue::new();
    let compiled = compile_workflow(&definition, &mut catalogue)?;

    let mut pre_supplied = parse_workflow_input_flags(&compiled, args, id_index)?;

    // The global `--namespace` flag feeds a `namespace` workflow input when
    // the workflow declares one and no per-workflow `--namespace` was given.
    seed_namespace_input(&mut pre_supplied, &compiled, flags.namespace.as_ref());

    execute_compiled_workflow(
        compiled,
        pre_supplied,
        flags,
        render_options,
        frontend_context,
        workflow_is_bundled,
        outdated_warning,
        telemetry_task,
    )
    .await
}

/// Shared back half of `route_workflow_run` (and the `extend docker-login`
/// default path). Everything from the runtime prologue onward: resolve
/// auth/base URL, build surfaces, drive the executor.
///
/// The front half (argv parsing, help, compilation) is route-specific; this
/// function is the generic "drive a compiled workflow" lifecycle that does
/// not touch argv or raw args.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_compiled_workflow(
    compiled: ags_protocol::workflow::CompiledWorkflow,
    mut pre_supplied: std::collections::BTreeMap<String, serde_json::Value>,
    flags: &GlobalFlags,
    render_options: crate::frontend::RenderOptions,
    frontend_context: &crate::invocation::context::FrontendContext,
    workflow_is_bundled: bool,
    outdated_warning: Option<String>,
    telemetry_task: Option<
        tokio::task::JoinHandle<
            Option<(
                String,
                String,
                ags_runtime::runtime::telemetry::TelemetryClient,
            )>,
        >,
    >,
) -> Result<InvocationOutcome, CliError> {
    // Runtime prologue (shared with `route_service`): resolves auth/base URL and
    // renders access-token warnings on a fresh pre-surface frontend, all before
    // any owned phase surface exists. A failure returns `Err` for the caller's
    // fresh human frontend to render.
    let (context, http_client) =
        super::run_prologue(flags, frontend_context, &render_options).await?;
    // Fall back to the fully-resolved namespace for the workflow's `namespace`
    // input. `context.namespace` incorporates the active profile's configured
    // namespace (and env), not just the raw `--namespace` flag handled above, so
    // a profile-configured namespace populates the input (and any dynamic-enum
    // pickers that depend on it) without the user re-typing it.
    seed_namespace_input(&mut pre_supplied, &compiled, context.namespace.as_ref());

    let mut runtime =
        ags_runtime::runtime::Runtime::from_reqwest(context.clone(), http_client.clone());

    let pagination = flags.pagination_hint();
    let mut options = RunOptions {
        dry_run: flags.is_dry_run,
        assume_yes: flags.is_auto_confirmed,
        no_input: !frontend_context.allows_input(),
        // Set below, once `finalize_surface` has resolved the real surface.
        review_steps: false,
        // Set below, alongside review_steps, once the real surface is known.
        pickers_available: false,
        output_format: frontend_context.protocol_output_format(),
        output: flags.output.clone(),
        verbosity: flags.verbosity,
        pagination,
        explicit_body: None,
        is_bundled_workflow: workflow_is_bundled,
    };

    // The prologue succeeded — construct the workflow surfaces now. For the
    // inline-terminal surface this is the single terminal acquisition; doing
    // it here means a failed prologue never grabs the terminal.
    //
    // Decision matrix: `finalize_surface` applies the matrix now
    // that the route (Workflow) and shape are known. `--ui=plain` /
    // `--ui=inline` override the default fullscreen surface; automation
    // consumers (`--format=json`) keep their machine-readable contract.
    //
    // `Shape::Multi` is a placeholder: `base_surface(Workflow, _)` ignores the
    // shape entirely (a workflow route is always Fullscreen unless overridden),
    // so the workflow's real step count never affects the surface.
    let frontend_context = frontend_context.finalize_surface(RouteKind::Workflow, Shape::Multi);
    // Help exits before finalization, so a workflow command here is never meta.
    crate::invocation::try_emit_first_run_hint(&frontend_context, false);
    let is_fullscreen_surface = matches!(
        frontend_context.surface_backend(),
        crate::invocation::context::PhaseBackend::FullscreenTerminalUi
    );
    // Plain/Inline: emit now, same timing as before. Fullscreen: hold it —
    // writing now would land before alt-screen acquisition and be lost;
    // `deferred_warning` is flushed after teardown, once the normal screen
    // buffer is restored (see below, after `run_phase_owned_execution`).
    if !is_fullscreen_surface {
        if let Some(msg) = &outdated_warning {
            let styled =
                crate::frontend::style::warning(msg, crate::frontend::style::is_stderr_enabled());
            let _ = crate::frontend::streams::UiSink.write_all(format!("{styled}\n").as_bytes());
        }
    }
    let deferred_warning = if is_fullscreen_surface {
        outdated_warning
    } else {
        None
    };
    // Per-step request review pauses on every step with its full editable
    // request. It applies only to an interactive fullscreen workflow run;
    // `--yes`, `--no-input`, and `--ui=plain`/`inline` keep the gather path.
    options.review_steps = review_steps_for(
        frontend_context.surface_backend(),
        options.assume_yes,
        options.no_input,
    );
    options.pickers_available = pickers_available_for(frontend_context.surface_backend());
    crate::invocation::register_reporter_if_plain(&frontend_context);
    let surfaces = if matches!(
        frontend_context.surface_backend(),
        crate::invocation::context::PhaseBackend::FullscreenTerminalUi
    ) {
        use crate::frontend::terminal::fullscreen::step_strip::{Step, StepRowKind, StepState};
        let mut fullscreen_steps: Vec<Step> = Vec::with_capacity(compiled.steps.len() + 1);
        // Prepend the "step 0 — Inputs" row only when Phase 1 will actually run
        // (interactive walk). With --yes/--no-input on fullscreen there is no
        // inputs phase, so no Inputs row.
        if options.review_steps {
            fullscreen_steps.push(Step {
                kind: StepRowKind::Inputs,
                title: "gather-inputs".into(),
                state: StepState::Pending,
            });
        }
        fullscreen_steps.extend(compiled.steps.iter().map(|s| Step {
            kind: StepRowKind::Workflow {
                runtime_index: s.index,
            },
            // Strip row text is the kebab step id (short, identifier-style); the
            // longer description renders inside the step's review panel.
            title: s.id.clone(),
            state: StepState::Pending,
        }));
        let resolver_context = context.clone();
        let resolver_client = http_client.clone();
        crate::frontend::select_fullscreen_workflow_surfaces(
            &frontend_context,
            render_options,
            compiled.name.clone(),
            crate::frontend::terminal::fullscreen::step_strip::HeaderKind::Workflow,
            fullscreen_steps,
            compiled.description.clone(),
            move |surface| {
                let resolver_runtime =
                    ags_runtime::runtime::Runtime::from_reqwest(resolver_context, resolver_client);
                Some(
                    Box::new(crate::frontend::dynamic_options::ProductionResolver::new(
                        resolver_runtime,
                        tokio::runtime::Handle::current(),
                        surface,
                    ))
                        as Box<dyn crate::frontend::dynamic_options::DynamicOptionResolver>,
                )
            },
        )?
    } else {
        let fetch: Option<Box<dyn crate::frontend::dynamic_options::OptionsFetch>> = if matches!(
            frontend_context.surface_backend(),
            crate::invocation::context::PhaseBackend::InlineTerminalUi
        ) {
            let fetch_runtime =
                ags_runtime::runtime::Runtime::from_reqwest(context.clone(), http_client.clone());
            Some(Box::new(
                crate::frontend::dynamic_options::RuntimeOptionsFetch::new(
                    fetch_runtime,
                    tokio::runtime::Handle::current(),
                ),
            ))
        } else {
            None
        };
        crate::frontend::select_workflow_phase_surfaces(
            &frontend_context,
            render_options,
            None,
            fetch,
        )?
    };

    // The shared post-prologue helper owns `RunStarted` onward (execution,
    // classification, `RunFinished`, teardown, final render). `FullLifecycle`
    // forwards workflow lifecycle banners — this is a registered workflow.
    let telemetry = match telemetry_task {
        Some(task) => task.await.ok().flatten().map(|(run_id, sub, client)| {
            build_workflow_step_telemetry(
                sub,
                client,
                &run_id,
                compiled.id.as_str(),
                compiled.steps.len(),
                options.dry_run,
                frontend_context.surface_backend().telemetry_label(),
            )
        }),
        None => None,
    };

    let phase_result = run_phase_owned_execution(
        surfaces,
        &compiled,
        pre_supplied,
        &mut runtime,
        &options,
        AdapterMode::FullLifecycle { telemetry },
        None,
    )
    .await;

    // Flushed here, after `run_phase_owned_execution` has returned — by this
    // point `.finish()` has already run on every code path inside it (see
    // `phase_execution.rs`), so the alt-screen is already torn down and this
    // lands in the normal buffer the user is looking at.
    if let Some(msg) = deferred_warning {
        let styled =
            crate::frontend::style::warning(&msg, crate::frontend::style::is_stderr_enabled());
        let _ = crate::frontend::streams::UiSink.write_all(format!("{styled}\n").as_bytes());
    }

    let (outcome, telemetry_client) = phase_result?;

    // Flush the exact `TelemetryClient` instance that queued the step events
    // (if any were queued) — `Drop` alone would not: `posthog-rs` sends on a
    // background worker, and this process may `std::process::exit` before
    // that worker is scheduled (the same reason `emit_with_outcome` always
    // flushes explicitly for the parent `cli.command.invoked` event). Bounded
    // by `flush_step_telemetry`'s timeout so a stalled network connection
    // cannot hang this already-completed command's exit indefinitely — the
    // parent `cli.command.invoked` flush in `emit_with_outcome` is a
    // deliberately separate, unbounded path and is out of scope here.
    flush_step_telemetry(telemetry_client).await;

    Ok(outcome)
}

/// Parse the workflow's own `--<input>` flags (everything after the id) into a
/// `pre_supplied` map, coercing each raw value against its declared schema. An
/// unknown flag surfaces as a clap usage error.
fn parse_workflow_input_flags(
    compiled: &ags_protocol::workflow::CompiledWorkflow,
    args: &[String],
    id_index: usize,
) -> Result<std::collections::BTreeMap<String, serde_json::Value>, CliError> {
    let command = build_registered_workflow_clap(compiled);
    let argv = std::iter::once(compiled.id.as_str().to_string()).chain(
        args.iter()
            .enumerate()
            .filter(|(index, _)| *index != id_index)
            .map(|(_, arg)| arg.clone()),
    );
    let matches = command
        .try_get_matches_from(argv)
        .map_err(|error| CliError::Usage {
            message: crate::invocation::clap_helpers::strip_clap_prefix(&error.to_string()),
            metadata: None,
        })?;
    let mut pre_supplied = std::collections::BTreeMap::new();
    for spec in &compiled.inputs {
        if let Some(raw) = matches.get_one::<String>(&spec.name) {
            pre_supplied.insert(
                spec.name.clone(),
                coerce_cli_value(raw, spec.schema.as_ref()),
            );
        }
    }
    Ok(pre_supplied)
}

/// Seed the workflow's `namespace` input from `value` when the workflow declares
/// one and it is not already set. No-op otherwise. Called once for the
/// `--namespace` flag and once for the prologue-resolved namespace.
fn seed_namespace_input(
    pre_supplied: &mut std::collections::BTreeMap<String, serde_json::Value>,
    compiled: &ags_protocol::workflow::CompiledWorkflow,
    value: Option<&String>,
) {
    if !compiled.inputs.iter().any(|spec| spec.name == "namespace") {
        return;
    }
    if let std::collections::btree_map::Entry::Vacant(slot) =
        pre_supplied.entry("namespace".to_string())
    {
        if let Some(namespace) = value {
            slot.insert(serde_json::Value::String(namespace.clone()));
        }
    }
}

/// Whether a workflow run should pause on every step for full per-step
/// request review. True only on the interactive fullscreen surface with
/// neither `--yes` nor `--no-input`; every other surface keeps the existing
/// gather path (see the design's "Where the pause decision lives").
fn review_steps_for(
    backend: crate::invocation::context::PhaseBackend,
    assume_yes: bool,
    no_input: bool,
) -> bool {
    // Phase 1 (collect declared inputs up front) runs on every interactive
    // surface. The rich surfaces (fullscreen, inline) also do per-step review;
    // plain implements only `collect_workflow_inputs`, so its per-step
    // `review_step` is the no-op default — it gathers up front, then runs. JSON
    // (automation) never gathers interactively. `--yes`/`--no-input` opt out of
    // interaction on any surface.
    matches!(
        backend,
        crate::invocation::context::PhaseBackend::FullscreenTerminalUi
            | crate::invocation::context::PhaseBackend::InlineTerminalUi
            | crate::invocation::context::PhaseBackend::PlainTerminal
    ) && !assume_yes
        && !no_input
}

/// Whether the active surface can render a dynamic-enum picker. True only on the
/// fullscreen and inline surfaces; plain and JSON have no picker, so the executor
/// suppresses picker-support inputs there (see the picker-support design).
fn pickers_available_for(backend: crate::invocation::context::PhaseBackend) -> bool {
    matches!(
        backend,
        crate::invocation::context::PhaseBackend::FullscreenTerminalUi
            | crate::invocation::context::PhaseBackend::InlineTerminalUi
    )
}

/// Handle `ags workflow list` — render the registered-workflow catalogue.
/// Offline: no runtime prologue, no auth.
fn route_workflow_list(
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    let entries: Vec<ags_protocol::workflow::WorkflowListEntry> = registry()
        .entries()
        .into_iter()
        .map(|(id, name)| ags_protocol::workflow::WorkflowListEntry { id, name })
        .collect();
    let output = ags_protocol::output::CommandOutput::WorkflowCatalogue { entries };
    frontend.render(&output)?;
    Ok(InvocationOutcome::Complete)
}

/// Handle `ags workflow add <path> [--validate-only]` — validate (and unless
/// `--validate-only`, install) a workflow YAML file. Offline: no runtime
/// prologue, no auth.
fn route_workflow_add(
    path: &std::path::Path,
    validate_only: bool,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    let runtime = ags_runtime::runtime::Runtime::from_reqwest(
        ags_runtime::runtime::execution::ExecutionContext::default(),
        ags_runtime::runtime::dispatch::http::build_http_client(None)?,
    );
    let view = runtime.workflow_add(path, validate_only)?;
    let output = ags_protocol::output::CommandOutput::WorkflowAdd(view);
    frontend.render(&output)?;
    Ok(InvocationOutcome::Complete)
}

/// Handle `ags workflow remove <id>` — delete a previously-installed external
/// workflow YAML file. Offline: no runtime prologue, no auth.
fn route_workflow_remove(
    id: &str,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    let runtime = ags_runtime::runtime::Runtime::from_reqwest(
        ags_runtime::runtime::execution::ExecutionContext::default(),
        ags_runtime::runtime::dispatch::http::build_http_client(None)?,
    );
    let view = runtime.workflow_remove(id)?;
    let output = ags_protocol::output::CommandOutput::WorkflowRemove(view);
    frontend.render(&output)?;
    Ok(InvocationOutcome::Complete)
}

/// Handle `ags workflow template [--output <path>]` — emit a starter workflow
/// YAML skeleton, either to stdout or a file. Offline: no runtime prologue, no
/// auth.
fn route_workflow_template(
    output_path: Option<&std::path::Path>,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    let runtime = ags_runtime::runtime::Runtime::from_reqwest(
        ags_runtime::runtime::execution::ExecutionContext::default(),
        ags_runtime::runtime::dispatch::http::build_http_client(None)?,
    );
    let view = runtime.workflow_template(output_path)?;
    let output = ags_protocol::output::CommandOutput::WorkflowTemplate(view);
    frontend.render(&output)?;
    Ok(InvocationOutcome::Complete)
}

/// Render `ags workflow run [<id>] --help` without running the auth-
/// requiring runtime prologue. With a registered id, compiles the workflow
/// against a bare `Catalogue` and prints its input-derived `--<flag>` help.
fn render_workflow_run_help(workflow_id: Option<&str>) -> Result<InvocationOutcome, CliError> {
    let Some(id) = workflow_id else {
        // No id: render the `run` subcommand's clap help (styled, with the
        // `<workflow-id>` argument under Arguments) instead of a hand-written
        // string, so it follows the same convention as the rest of the CLI.
        let command = crate::invocation::builder::build_workflow_command();
        if let Some(run) = command.find_subcommand("run") {
            let mut run = run.clone().bin_name("ags workflow run");
            write_command_help(&mut run);
        }
        return Ok(InvocationOutcome::Complete);
    };
    let definition = match registry().resolve(&WorkflowId::new(id)) {
        Some(workflow) => workflow.definition().clone(),
        None => return Err(unknown_workflow_error(id)),
    };
    let mut catalogue = ags_runtime::catalogue::Catalogue::new();
    let compiled = compile_workflow(&definition, &mut catalogue)?;
    let mut command =
        build_registered_workflow_clap(&compiled).bin_name(format!("ags workflow run {id}"));
    write_command_help(&mut command);
    Ok(InvocationOutcome::Complete)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invocation::context::{
        ConsumerKind, FrontendContext, InteractionPolicy, TerminalCapabilities,
    };
    use ags_protocol::request::OutputFormat;

    #[test]
    fn test_workflow_help_omits_registered_workflows_block() {
        // The registered-workflow list belongs to `ags workflow list`, not the
        // `--help` output — the duplicated block was removed.
        let mut command = crate::invocation::builder::build_workflow_command();
        let help = command.render_long_help().to_string();
        assert!(
            !help.contains("Registered workflows"),
            "the registered-workflows block should be gone from --help"
        );
        assert!(help.contains("run"), "run subcommand still listed");
        assert!(help.contains("list"), "list subcommand still listed");
    }

    #[test]
    fn test_workflow_help_ansi_render_is_styled() {
        // The styled (`.ansi()`) render carries ANSI escapes so headings and
        // commands render bold/underlined like the rest of the CLI; plain
        // `.to_string()` (used when colour is disabled) does not.
        let mut command = crate::invocation::builder::build_workflow_command();
        let ansi = command.render_long_help().ansi().to_string();
        assert!(
            ansi.contains('\u{1b}'),
            "styled help should contain ANSI escape codes"
        );
    }

    #[test]
    fn test_review_steps_only_for_fullscreen_interactive() {
        use crate::invocation::context::PhaseBackend::*;
        assert!(review_steps_for(FullscreenTerminalUi, false, false));
        assert!(!review_steps_for(FullscreenTerminalUi, true, false)); // --yes
        assert!(!review_steps_for(FullscreenTerminalUi, false, true)); // --no-input
        assert!(review_steps_for(PlainTerminal, false, false)); // --ui=plain: gather up front
        assert!(!review_steps_for(PlainTerminal, true, false)); // plain --yes
        assert!(!review_steps_for(PlainTerminal, false, true)); // plain --no-input
        assert!(review_steps_for(InlineTerminalUi, false, false)); // --ui=inline: full flow
        assert!(!review_steps_for(InlineTerminalUi, true, false)); // inline --yes
        assert!(!review_steps_for(InlineTerminalUi, false, true)); // inline --no-input
        assert!(!review_steps_for(StructuredJson, false, false)); // --format=json
    }

    #[test]
    fn test_pickers_available_for_fullscreen_and_inline() {
        use crate::invocation::context::PhaseBackend::*;
        // Fullscreen and inline both render the dynamic-enum picker now.
        assert!(pickers_available_for(FullscreenTerminalUi));
        assert!(pickers_available_for(InlineTerminalUi));
        // Plain and JSON have no picker.
        assert!(!pickers_available_for(PlainTerminal));
        assert!(!pickers_available_for(StructuredJson));
    }

    /// Build a default human `FrontendContext` for tests (allows input, non-automation).
    fn human_frontend_context() -> FrontendContext {
        FrontendContext {
            consumer: ConsumerKind::Human,
            interaction: InteractionPolicy {
                allow_input: true,
                prefer_rich_ui: false,
                prefer_fullscreen: false,
            },
            terminal: TerminalCapabilities {
                stdin_is_tty: false,
                stdout_is_tty: false,
                stderr_is_tty: false,
                color_force_off: true,
            },
            ui_intent: crate::invocation::flags::UiFlag::Auto,
        }
    }

    #[tokio::test]
    async fn test_route_workflow_run_unknown_id_errors() {
        let flags = GlobalFlags::default();
        let ctx = human_frontend_context();
        let result = route_workflow_run(
            &["definitely-not-a-real-workflow".to_string()],
            &flags,
            crate::frontend::RenderOptions::default(),
            &ctx,
            None,
        )
        .await;
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert!(
                    message.contains("Unknown workflow: 'definitely-not-a-real-workflow'"),
                    "got: {message}"
                );
                // Mirrors the `Unknown service` error: lists valid options.
                assert!(message.contains("Valid workflows:"), "got: {message}");
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_route_workflow_run_json_automation_not_rejected_up_front() {
        // Automation (--format json) must no longer be rejected before the
        // registry lookup. With an unknown id it now reaches the registry and
        // returns the unknown-workflow error — NOT the old format rejection.
        let flags = GlobalFlags {
            format: Some(OutputFormat::Json),
            ..GlobalFlags::default()
        };
        let ctx = FrontendContext {
            consumer: ConsumerKind::Automation,
            interaction: InteractionPolicy {
                allow_input: false,
                prefer_rich_ui: false,
                prefer_fullscreen: false,
            },
            terminal: TerminalCapabilities {
                stdin_is_tty: false,
                stdout_is_tty: false,
                stderr_is_tty: false,
                color_force_off: true,
            },
            ui_intent: crate::invocation::flags::UiFlag::Auto,
        };
        let result = route_workflow_run(
            &["definitely-not-a-real-workflow".to_string()],
            &flags,
            crate::frontend::RenderOptions::default(),
            &ctx,
            None,
        )
        .await;
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert!(
                    message.contains("Unknown workflow"),
                    "automation should reach the registry lookup, got: {message}"
                );
                assert!(
                    !message.contains("--format=json"),
                    "the old format rejection must be gone: {message}"
                );
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_route_workflow_run_rejects_skeleton() {
        let flags = GlobalFlags {
            is_skeleton: true,
            ..GlobalFlags::default()
        };
        let ctx = human_frontend_context();
        let result = route_workflow_run(
            &["anything".to_string()],
            &flags,
            crate::frontend::RenderOptions::default(),
            &ctx,
            None,
        )
        .await;
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert!(message.contains("--skeleton"), "got: {message}");
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_route_workflow_run_help_renders_for_registered_id() {
        let flags = GlobalFlags::default();
        let ctx = human_frontend_context();
        let result = route_workflow_run(
            &["competitive-multiplayer".to_string(), "--help".to_string()],
            &flags,
            crate::frontend::RenderOptions::default(),
            &ctx,
            None,
        )
        .await;
        assert!(
            matches!(result, Ok(InvocationOutcome::Complete)),
            "got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_route_workflow_run_help_before_id_renders() {
        let flags = GlobalFlags::default();
        let ctx = human_frontend_context();
        let result = route_workflow_run(
            &["--help".to_string(), "competitive-multiplayer".to_string()],
            &flags,
            crate::frontend::RenderOptions::default(),
            &ctx,
            None,
        )
        .await;
        assert!(
            matches!(result, Ok(InvocationOutcome::Complete)),
            "got: {result:?}"
        );
    }

    #[test]
    fn test_build_workflow_step_telemetry_maps_fields_from_inputs() {
        let sink = build_workflow_step_telemetry(
            "user-sub-123".to_string(),
            ags_runtime::runtime::telemetry::TelemetryClient::disabled_for_test(),
            "run-1",
            "competitive-multiplayer",
            4,
            true,
            "fullscreen",
        );

        assert_eq!(sink.sub, "user-sub-123");
        assert_eq!(sink.context.run_id, "run-1");
        assert_eq!(sink.context.workflow_id, "competitive-multiplayer");
        assert_eq!(sink.context.steps_total, 4);
        assert_eq!(sink.context.cli_version, env!("CARGO_PKG_VERSION"));
        assert!(sink.context.is_dry_run);
        assert_eq!(sink.context.ui_surface, "fullscreen");
    }

    #[tokio::test]
    async fn test_route_workflow_run_unknown_flag_errors_before_prologue() {
        // An unknown workflow flag must surface as a clap usage error from
        // the pre-prologue parse — not fail late on auth/base-URL.
        let flags = GlobalFlags::default();
        let ctx = human_frontend_context();
        let result = route_workflow_run(
            &["competitive-multiplayer".to_string(), "--bogus".to_string()],
            &flags,
            crate::frontend::RenderOptions::default(),
            &ctx,
            None,
        )
        .await;
        assert!(
            matches!(result, Err(CliError::Usage { .. })),
            "got: {result:?}"
        );
    }

    #[test]
    fn test_is_workflow_run_matches_workflow_run() {
        let remaining = vec!["workflow".to_string(), "run".to_string(), "x".to_string()];
        assert!(is_workflow_run(&remaining));
    }

    #[test]
    fn test_is_workflow_run_rejects_workflow_list() {
        let remaining = vec!["workflow".to_string(), "list".to_string()];
        assert!(!is_workflow_run(&remaining));
    }

    #[test]
    fn test_is_workflow_run_rejects_bare_workflow() {
        let remaining = vec!["workflow".to_string()];
        assert!(!is_workflow_run(&remaining));
    }

    #[test]
    fn test_is_workflow_run_rejects_other_command() {
        let remaining = vec!["iam".to_string(), "run".to_string()];
        assert!(!is_workflow_run(&remaining));
    }

    #[test]
    fn test_is_workflow_run_rejects_empty() {
        assert!(!is_workflow_run(&[]));
    }

    #[test]
    fn test_mismatched_protocol_version_warning_message_format() {
        let msg = mismatched_protocol_version_warning("my-workflow", "0.3.0", "1.0.0");
        assert_eq!(
            msg,
            "Workflow 'my-workflow' targets protocol version 0.3.0; this ags build is on \
             protocol version 1.0.0. If you encounter issues, this may be why."
        );
    }

    #[test]
    fn test_unreadable_protocol_version_warning_message_format() {
        let msg = unreadable_protocol_version_warning("my-workflow", "banana", "1.0.0");
        assert!(
            msg.contains("banana"),
            "must name the declared value: {msg}"
        );
        assert!(
            msg.contains("1.0.0"),
            "must name the current protocol version: {msg}"
        );
        assert!(
            msg.contains("not a readable version number"),
            "must state what is wrong: {msg}"
        );
    }

    #[test]
    fn test_legacy_protocol_version_warning_message_format() {
        let msg = legacy_protocol_version_warning("my-workflow", "1.0.0");
        assert_eq!(
            msg,
            "Workflow 'my-workflow' does not declare a protocol version (likely installed \
             before this CLI started requiring one); this ags build is on protocol version \
             1.0.0. If you encounter issues, this may be why."
        );
    }
}
