//! Trusted, proof-based reconciliation of uncertain native operations.
//!
//! No MCP tool can clear a recovery freeze. A missing journal row is not proof
//! that writing never happened; only an accepted-before-write phase fence or a
//! correlated native completion/strict Undo can establish a terminal outcome.
use super::*;
use tagrex_commands::RecoveryBatchDto;

struct Evidence {
    record: OperationRecord,
    batches: Vec<RecoveryBatchDto>,
    damaged: bool,
    root_matches: bool,
    proof: String,
    action: Option<&'static str>,
    guidance: &'static str,
}
impl Service {
    /// Fast empty response on the ordinary path: status/Stop never wait for a
    /// running writer's App mutex merely to discover there is no recovery.
    pub fn recovery_report(&self) -> ServiceResult<Value> {
        let records = {
            let state = lock(&self.0.state)?;
            if !state.recovery_required {
                return Ok(json!({"recovery_required":false,"operations":[]}));
            }
            state
                .operations
                .values()
                .filter_map(|operation| {
                    let record = operation.record.lock().ok()?;
                    (record.status == "recovery_required").then(|| record.clone())
                })
                .take(100)
                .collect::<Vec<_>>()
        };
        let app_guard = lock(&self.0.app).map_err(|_| {
            ServiceError::new(
                "restart_required",
                "The engine stopped unexpectedly; restart TAGnetic before inspecting its journal",
            )
        })?;
        let app = app_guard.as_ref().ok_or_else(no_library)?;
        let mut reports = Vec::new();
        for record in records {
            let evidence = self.recovery_evidence(app, record)?;
            reports.push(json!({"operation":evidence.record.id,"principal_id":evidence.record.owner,"phase":evidence.record.interrupted_phase,"proof":evidence.proof,"action":evidence.action,"guidance":evidence.guidance,"batches":evidence.batches.iter().map(|batch|json!({"id":batch.id,"description":batch.description,"interrupted":batch.interrupted,"files":batch.paths.len(),"paths":batch.paths.iter().take(100).collect::<Vec<_>>(),"truncated":batch.paths.len()>100})).collect::<Vec<_>>()}));
        }
        Ok(json!({"recovery_required":true,"operations":reports}))
    }
    /// Called only by a trusted desktop command after showing the evidence and
    /// exact action. Rechecks journal facts while holding the mutation owner.
    pub fn reconcile_operation(
        &self,
        operation_id: &str,
        proof: &str,
        action: &str,
    ) -> ServiceResult<Value> {
        let _mutation = lock(&self.0.mutation).map_err(|_| {
            ServiceError::new(
                "restart_required",
                "Restart TAGnetic before reconciling a panicked write worker",
            )
        })?;
        let mut app_guard = lock(&self.0.app).map_err(|_| {
            ServiceError::new(
                "restart_required",
                "Restart TAGnetic before reconciling a panicked engine",
            )
        })?;
        let app = app_guard.as_mut().ok_or_else(no_library)?;
        let operation = {
            let state = lock(&self.0.state)?;
            state
                .operations
                .get(operation_id)
                .cloned()
                .ok_or_else(missing_handle)?
        };
        let record = lock(&operation.record)?.clone();
        if record.status != "recovery_required" {
            return Err(ServiceError::conflict(
                "Operation no longer requires recovery; refresh its status",
            ));
        }
        let evidence = self.recovery_evidence(app, record)?;
        if evidence.proof != proof || evidence.action != Some(action) {
            return Err(ServiceError::conflict(
                "Recovery evidence or available action changed; inspect it again",
            ));
        }
        if evidence.damaged || !evidence.root_matches {
            return Err(ServiceError::new("recovery_required", evidence.guidance));
        }
        // Persist the chosen bounded recovery action before a possible native
        // Undo. A second crash remains uncertain and never triggers replay.
        {
            let mut state = lock(&self.0.state)?;
            {
                let mut record = lock(&operation.record)?;
                record.interrupted_phase = Some(format!("recovering_{action}"));
            }
            super::plans::persist_recovery_operations(&self.0.directory, &state)?;
            state.recovery_required = true;
        }
        let result = match action {
            "accept_completed" => {
                let batch = evidence.batches.first().ok_or_else(missing_handle)?;
                json!({"batch":{"id":batch.id,"description":batch.description},"files":batch.paths.len(),"recovered":true})
            }
            "undo_correlated" => {
                let batch = evidence.batches.first().ok_or_else(missing_handle)?;
                let report = app.undo(batch.id)?;
                if !report.kept.is_empty() {
                    return Err(ServiceError::new(
                        "recovery_required",
                        "Strict recovery Undo retained later values; inspect the files and journal",
                    ));
                }
                let (_, remaining) = app.recovery_batches(&evidence.record.plan_id)?;
                if !remaining.is_empty() {
                    return Err(ServiceError::new(
                        "recovery_required",
                        "Native journal still retains the operation after Undo",
                    ));
                }
                self.emit("session_changed", json!({"refresh":true}));
                json!({"restored":true,"report":report,"recovered":true})
            }
            "close_not_written" => json!({"written":false,"phase_fence":true,"recovered":true}),
            _ => return Err(ServiceError::invalid("Unsupported recovery action")),
        };
        let mut state = lock(&self.0.state)?;
        {
            let mut record = lock(&operation.record)?;
            record.status = match action {
                "accept_completed" => "succeeded_recovered",
                "undo_correlated" => "restored_recovered",
                _ => "failed_before_write",
            }
            .into();
            record.result = Some(result.clone());
            record.error = None;
        }
        // The terminal receipt is durable before other writes can be unfrozen.
        super::plans::persist_recovery_operations(&self.0.directory, &state)?;
        state.recovery_required = state.operations.values().any(|op| {
            op.record
                .lock()
                .map(|r| r.status == "recovery_required")
                .unwrap_or(true)
        });
        drop(state);
        drop(app_guard);
        self.emit("operation_changed", json!({"reconciled":operation_id}));
        Ok(json!({"operation":operation_id,"result":result}))
    }
    fn recovery_evidence(&self, app: &App, record: OperationRecord) -> ServiceResult<Evidence> {
        let (damaged, batches) = app.recovery_batches(&record.plan_id)?;
        let current_root = fs::canonicalize(app.library_root())?;
        let root_matches = record
            .library_root
            .as_ref()
            .is_some_and(|root| root == &current_root);
        let phase = record.interrupted_phase.as_deref().unwrap_or("unknown");
        let (action, guidance) = if damaged {
            (None,"The journal was replaced after damage. Restore and inspect the preserved journal; a missing row cannot establish a safe outcome.")
        } else if !root_matches {
            (None,"Open the operation's original folder before inspecting its native journal evidence.")
        } else if batches.len() > 1 {
            (None,"More than one journal batch matches this operation. Inspect the journal; automatic reconciliation is ambiguous.")
        } else if let Some(batch) = batches.first() {
            if batch.interrupted {
                (Some("undo_correlated"),"The native journal retains an interrupted batch. Strict Undo can restore only the changes it can prove ownership of.")
            } else if matches!(phase, "writing" | "recovering_accept_completed") {
                (Some("accept_completed"),"The exact server-issued draft marker has one completed native journal batch. Record that committed outcome without writing files again.")
            } else {
                (Some("undo_correlated"),"A correlated batch remains after an uncertain history action. Use strict native Undo; completion alone does not prove the current file values.")
            }
        } else if phase == "queued" {
            (Some("close_not_written"),"The durable receipt never crossed the writing phase fence and no matching native batch exists. Close it as not written.")
        } else {
            (None,"No matching journal batch remains after writing may have started. Missing history is not restoration proof; keep this operation frozen and inspect preserved recovery evidence.")
        };
        let proof = hash(&serde_json::to_vec(
            &json!({"epoch":self.0.epoch,"operation":record.id,"draft":record.plan_id,"phase":phase,"root":current_root,"damaged":damaged,"batches":batches}),
        )?);
        Ok(Evidence {
            record,
            batches,
            damaged,
            root_matches,
            proof,
            action,
            guidance,
        })
    }
}
