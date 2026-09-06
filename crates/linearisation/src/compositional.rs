//! Milestone 2 of the linearisation port (see
//! `docs/linearisation-implementation-plan.md`): recognising a top-level
//! `init` shaped as `hide(H, allow(A, comm(C, P1(...) || ... || Pn(...))))`
//! (each of `hide`/`allow`/`comm` optional, in that fixed nesting order —
//! the same order [`crate`]'s own README and `crates/explore::combine_lts`
//! apply them in) and linearising each `Pi` independently.
//!
//! There is deliberately no "cleave" step here: `Pi` already *is* its own
//! process, not a fragment of one big already-linearised LPS that needs to be
//! split back apart (that is what mCRL2's `lpscleave` does, for a different
//! problem — see the crate README). So every `Pi` is linearised by exactly
//! [`crate::translate::linearise_process`], the same per-process core
//! [`crate::translate::linearise`] itself uses — which is also why a `Pi`
//! that internally contains another `||`/`allow`/`comm` is rejected for
//! free, with the same [`LinearisationError::UnsupportedConstruct`] it
//! would get outside a compositional context: nothing here specially checks
//! for that, the shared translator already refuses it.

use std::collections::HashMap;

use itertools::Itertools;
use merc_syntax::ProcDecl;
use merc_syntax::ProcExprBinaryOp;
use merc_syntax::ProcessExpr;
use merc_syntax::ProcessExprKind;
use merc_typecheck::ProcessSpecification;
use serde::Serialize;

use crate::LinearProcessSpecification;
use crate::LinearisationError;
use crate::translate::linearise_process;
use crate::translate::resolve_process_call;

/// One operand of the top-level `||`, linearised on its own.
#[derive(Clone, Debug, PartialEq)]
pub struct CompositionalComponent {
    /// The process's own name, disambiguated with a `_2`, `_3`, ... suffix
    /// when the same process is instantiated more than once at the top
    /// level (e.g. `P(1) || P(2)`).
    pub name: String,
    pub lps: LinearProcessSpecification,
}

/// The result of [`linearise_compositional`]: one [`LinearProcessSpecification`]
/// per operand of `init`'s top-level `||`, plus the hide/allow/comm sets that
/// compose them. Composing them is deliberately *not* done here — that's
/// `crates/explore::combine_lts`'s job, one level down at the LTS level, once
/// each component has been explored into an LTS. [`Self::manifest`] hands
/// off to it via `tools/lts combine`'s `--manifest` option.
#[derive(Clone, Debug, PartialEq)]
pub struct CompositionalSpecification {
    pub components: Vec<CompositionalComponent>,
    /// `{a, b}`-shaped text accepted by `merc_syntax::parse_action_names`
    /// (braces included), or `None` when `init` has no `hide(...)` wrapper.
    pub hide: Option<String>,
    /// `{a|b, c}`-shaped text accepted by
    /// `merc_syntax::parse_allow_action_names`, or `None` when `init` has no
    /// `allow(...)` wrapper.
    pub allow: Option<String>,
    /// `{a|b -> c}`-shaped text accepted by
    /// `merc_syntax::parse_comm_expr_set`, or `None` when `init` has no
    /// `comm(...)` wrapper.
    pub comm: Option<String>,
}

/// A `tools/lts combine --manifest`-ready description of a
/// [`CompositionalSpecification`]: each component's printed-LPS file name
/// alongside the same `hide`/`allow`/`comm` text. See
/// [`CompositionalSpecification::manifest`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Manifest {
    pub components: Vec<ManifestComponent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hide: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comm: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ManifestComponent {
    pub name: String,
    pub lps_file: String,
}

impl CompositionalSpecification {
    /// Builds a [`Manifest`], naming each component's file
    /// `"{component.name}.mcrl2"` — the component names are already
    /// disambiguated (see [`CompositionalComponent::name`]), so this can't
    /// collide. Serialize the result with `serde_json` to get the JSON file
    /// `tools/lts combine --manifest` reads.
    pub fn manifest(&self) -> Manifest {
        Manifest {
            components: self
                .components
                .iter()
                .map(|component| ManifestComponent {
                    name: component.name.clone(),
                    lps_file: format!("{}.mcrl2", component.name),
                })
                .collect(),
            hide: self.hide.clone(),
            allow: self.allow.clone(),
            comm: self.comm.clone(),
        }
    }
}

/// Linearises `spec`'s `init` compositionally: see the module documentation
/// for exactly what shape of `init` this recognises.
pub fn linearise_compositional(spec: &ProcessSpecification) -> Result<CompositionalSpecification, LinearisationError> {
    let init = spec.init().ok_or(LinearisationError::MissingInit)?;

    let mut rest = init;
    let hide = if let ProcessExprKind::Hide { actions, operand } = &rest.node {
        rest = operand;
        Some(format!("{{{}}}", actions.iter().format(", ")))
    } else {
        None
    };
    let allow = if let ProcessExprKind::Allow { actions, operand } = &rest.node {
        rest = operand;
        Some(format!("{{{}}}", actions.iter().format(", ")))
    } else {
        None
    };
    let comm = if let ProcessExprKind::Comm { comm, operand } = &rest.node {
        rest = operand;
        Some(format!("{{{}}}", comm.iter().format(", ")))
    } else {
        None
    };

    let processes: HashMap<&str, &ProcDecl> = spec
        .process_declarations()
        .iter()
        .map(|decl| (decl.identifier.node.as_str(), decl))
        .collect();

    let mut operands = Vec::new();
    flatten_parallel(rest, &mut operands);

    let calls = operands
        .into_iter()
        .map(|operand| {
            resolve_process_call(operand, &processes)?.ok_or_else(|| {
                LinearisationError::CompositionalOperandNotAProcessCall {
                    span: operand.span.clone(),
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut occurrences: HashMap<&str, usize> = HashMap::new();
    for (decl, _) in &calls {
        *occurrences.entry(decl.identifier.node.as_str()).or_insert(0) += 1;
    }

    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut components = Vec::with_capacity(calls.len());
    for (decl, args) in calls {
        let base = decl.identifier.node.as_str();
        let name = if occurrences[base] > 1 {
            let count = seen.entry(base).or_insert(0);
            *count += 1;
            format!("{base}_{count}")
        } else {
            base.to_string()
        };

        let lps = linearise_process(decl.params.clone(), args, Some(base), &decl.body, &processes)?;
        components.push(CompositionalComponent { name, lps });
    }

    Ok(CompositionalSpecification {
        components,
        hide,
        allow,
        comm,
    })
}

/// Flattens a left/right-nested `Binary{Parallel}` tree into its leaves, in
/// left-to-right order.
fn flatten_parallel<'a>(expr: &'a ProcessExpr, out: &mut Vec<&'a ProcessExpr>) {
    match &expr.node {
        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Parallel,
            lhs,
            rhs,
        } => {
            flatten_parallel(lhs, out);
            flatten_parallel(rhs, out);
        }
        _ => out.push(expr),
    }
}
