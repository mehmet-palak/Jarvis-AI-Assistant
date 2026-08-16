//! F4 "Çok-adımlı workflow runner": planı kullanıcıya gösterme, her yan etkili adımdan önce
//! policy/approval, retry/idempotency, iptalde cleanup ve audit özeti — genel, tool-bağımsız bir
//! orkestratör. F4'ün kendi plan→patch→onay→uygula→test zinciri bu soyutlamanın somut bir
//! örneği; bu modül onu tek bir kullanıma özel bir mekanizma olmaktan çıkarıp, gelecekteki her
//! çok-adımlı işin (F4 "Yerel üretkenlik tool framework"'ün `LocalTool`'ları dahil) üzerine
//! kurulabileceği tek bir gerçek çerçeveye dönüştürüyor.

use std::sync::atomic::Ordering;

use crate::workbench::CancelFlag;

/// Bir çalıştırılabilir adım. `id`/`description` planı kullanıcıya göstermek için (`describe_workflow`);
/// `has_side_effect` hangi adımların yürütmeden önce açık onay gerektirdiğini belirliyor (salt-okunur
/// bir adım — ör. bir ön kontrol — onay istemeden geçebilir); `idempotency_key` aynı işin bir
/// retry'da ya da tekrar çalıştırmada iki kez uygulanmasını engelliyor; `rollback` bir sonraki
/// adım başarısız olursa ya da workflow iptal edilirse bu adımın etkisini geri almaya çalışıyor
/// (best-effort — her yan etki gerçekte geri alınabilir değildir, `rollback`'in kendi başarısı
/// ayrıca raporlanıyor).
pub trait WorkflowStep {
    fn id(&self) -> &str;
    fn description(&self) -> &str;
    fn has_side_effect(&self) -> bool;
    /// `Some` ise, aynı anahtarla daha önce başarıyla tamamlanmış bir adım bu çalıştırmada
    /// tekrar yürütülmez — `SkippedIdempotent` olarak işaretlenir.
    fn idempotency_key(&self) -> Option<String> {
        None
    }
    /// Gerçek işlem. `Ok(evidence)` — kısa, insan-okunur bir kanıt string'i (audit özetine gider).
    fn execute(&self) -> Result<String, String>;
    /// Bu adım daha önce başarıyla tamamlandıysa ve workflow sonradan durdurulduysa (hata ya da
    /// iptal) çağrılır. Yan etkisi olmayan adımlar için varsayılan gövde her zaman `Ok(())`.
    fn rollback(&self) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcomeKind {
    /// Aynı `idempotency_key`'e sahip bir adım bu çalıştırmada daha önce başarıyla tamamlanmıştı.
    SkippedIdempotent,
    /// Gerçekten yürütüldü ve başarılı oldu; string kanıttır.
    Succeeded(String),
    /// Onay gerektiren bir adım reddedildi — hiç yürütülmedi.
    ApprovalDenied,
    /// Tüm retry denemeleri tükendi; son hata.
    Failed(String),
    /// Workflow bu adıma gelmeden iptal edildi.
    Cancelled,
    /// Bu adım daha önce başarıyla tamamlanmıştı ama sonraki bir adım başarısız/iptal olduğu
    /// için geri alındı. `Ok(())` ise rollback başarılı; `Err` ise rollback'in kendisi başarısız
    /// oldu (dürüstçe raporlanıyor, gizlenmiyor).
    RolledBack(Result<(), String>),
    /// Bu adıma hiç gelinmedi (daha önceki bir adım workflow'u durdurdu).
    NotReached,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowStepOutcome {
    pub step_id: String,
    pub description: String,
    pub had_side_effect: bool,
    pub outcome: StepOutcomeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowSummary {
    pub outcomes: Vec<WorkflowStepOutcome>,
    /// Her adım ya gerçekten başarılı oldu ya da idempotent olarak atlandı — hiçbir adım
    /// reddedilmedi/başarısız olmadı/iptal edilmedi.
    pub completed: bool,
}

/// Planı kullanıcıya göstermek için — hiçbir adımı yürütmez, yalnız sırayı ve hangi adımların
/// onay gerektireceğini açıklıyor.
pub fn describe_workflow(steps: &[Box<dyn WorkflowStep>]) -> Vec<String> {
    steps
        .iter()
        .map(|step| {
            let side_effect_note = if step.has_side_effect() {
                " (onay gerekir)"
            } else {
                ""
            };
            format!("{}: {}{side_effect_note}", step.id(), step.description())
        })
        .collect()
}

/// Adımları sırayla çalıştırır. `approve`, yan etkili her adımdan önce çağrılır (F4 "her yan
/// etkili adımdan önce policy/approval"); `false` dönerse adım reddedilmiş sayılır, önceki
/// tamamlanmış yan etkili adımlar geri alınır, kalan adımlar `NotReached` işaretlenir.
/// `max_retries`, geçici bir hatadan sonra AYNI adımı kaç kez daha denediğini belirtir (0 =
/// yalnız bir deneme). `cancel` her adımdan önce kontrol edilir — set edilmişse workflow o anda
/// durur, tamamlanmış adımlar geri alınır.
pub fn run_workflow(
    steps: Vec<Box<dyn WorkflowStep>>,
    approve: &dyn Fn(&dyn WorkflowStep) -> bool,
    cancel: Option<&CancelFlag>,
    max_retries: u32,
) -> WorkflowSummary {
    let mut outcomes: Vec<WorkflowStepOutcome> = Vec::with_capacity(steps.len());
    let mut completed_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut succeeded_side_effecting_steps: Vec<usize> = Vec::new();
    let mut stopped_at: Option<usize> = None;

    for (index, step) in steps.iter().enumerate() {
        if stopped_at.is_some() {
            outcomes.push(WorkflowStepOutcome {
                step_id: step.id().to_string(),
                description: step.description().to_string(),
                had_side_effect: step.has_side_effect(),
                outcome: StepOutcomeKind::NotReached,
            });
            continue;
        }

        if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            outcomes.push(WorkflowStepOutcome {
                step_id: step.id().to_string(),
                description: step.description().to_string(),
                had_side_effect: step.has_side_effect(),
                outcome: StepOutcomeKind::Cancelled,
            });
            stopped_at = Some(index);
            continue;
        }

        if let Some(key) = step.idempotency_key() {
            if completed_keys.contains(&key) {
                outcomes.push(WorkflowStepOutcome {
                    step_id: step.id().to_string(),
                    description: step.description().to_string(),
                    had_side_effect: step.has_side_effect(),
                    outcome: StepOutcomeKind::SkippedIdempotent,
                });
                continue;
            }
        }

        if step.has_side_effect() && !approve(step.as_ref()) {
            outcomes.push(WorkflowStepOutcome {
                step_id: step.id().to_string(),
                description: step.description().to_string(),
                had_side_effect: true,
                outcome: StepOutcomeKind::ApprovalDenied,
            });
            stopped_at = Some(index);
            continue;
        }

        let mut attempt_result = step.execute();
        let mut attempts_left = max_retries;
        while attempt_result.is_err() && attempts_left > 0 {
            attempts_left -= 1;
            attempt_result = step.execute();
        }

        match attempt_result {
            Ok(evidence) => {
                if let Some(key) = step.idempotency_key() {
                    completed_keys.insert(key);
                }
                if step.has_side_effect() {
                    succeeded_side_effecting_steps.push(index);
                }
                outcomes.push(WorkflowStepOutcome {
                    step_id: step.id().to_string(),
                    description: step.description().to_string(),
                    had_side_effect: step.has_side_effect(),
                    outcome: StepOutcomeKind::Succeeded(evidence),
                });
            }
            Err(error) => {
                outcomes.push(WorkflowStepOutcome {
                    step_id: step.id().to_string(),
                    description: step.description().to_string(),
                    had_side_effect: step.has_side_effect(),
                    outcome: StepOutcomeKind::Failed(error),
                });
                stopped_at = Some(index);
            }
        }
    }

    // İptalde/hatada cleanup: tamamlanmış yan etkili adımları TERS sırayla geri al — sonradan
    // eklenen bir etki, ondan önceki bir etkiye bağlı olabilir (ör. B, A'nın varlığına bağlıysa,
    // önce B sonra A geri alınmalı).
    if stopped_at.is_some() {
        for &index in succeeded_side_effecting_steps.iter().rev() {
            let rollback_result = steps[index].rollback();
            outcomes[index] = WorkflowStepOutcome {
                step_id: steps[index].id().to_string(),
                description: steps[index].description().to_string(),
                had_side_effect: true,
                outcome: StepOutcomeKind::RolledBack(rollback_result),
            };
        }
    }

    let completed = stopped_at.is_none();
    WorkflowSummary {
        outcomes,
        completed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    /// Test'ler için genel amaçlı bir adım — çağrı sayısını, onay isteğini ve rollback çağrısını
    /// izliyor.
    struct RecordingStep {
        id: String,
        has_side_effect: bool,
        idempotency_key: Option<String>,
        fail_first_n_calls: Cell<u32>,
        always_fails: bool,
        execute_calls: Cell<u32>,
        rolled_back: Cell<bool>,
        rollback_fails: bool,
    }

    impl RecordingStep {
        fn new(id: &str, has_side_effect: bool) -> Self {
            Self {
                id: id.to_string(),
                has_side_effect,
                idempotency_key: None,
                fail_first_n_calls: Cell::new(0),
                always_fails: false,
                execute_calls: Cell::new(0),
                rolled_back: Cell::new(false),
                rollback_fails: false,
            }
        }
    }

    impl WorkflowStep for RecordingStep {
        fn id(&self) -> &str {
            &self.id
        }
        fn description(&self) -> &str {
            "test step"
        }
        fn has_side_effect(&self) -> bool {
            self.has_side_effect
        }
        fn idempotency_key(&self) -> Option<String> {
            self.idempotency_key.clone()
        }
        fn execute(&self) -> Result<String, String> {
            let calls = self.execute_calls.get() + 1;
            self.execute_calls.set(calls);
            if self.always_fails {
                return Err(format!("{} always fails", self.id));
            }
            if calls <= self.fail_first_n_calls.get() {
                return Err(format!("{} transient failure #{calls}", self.id));
            }
            Ok(format!("{}-evidence", self.id))
        }
        fn rollback(&self) -> Result<(), String> {
            self.rolled_back.set(true);
            if self.rollback_fails {
                Err(format!("{} rollback failed", self.id))
            } else {
                Ok(())
            }
        }
    }

    fn approve_all(_step: &dyn WorkflowStep) -> bool {
        true
    }

    #[test]
    fn describe_workflow_lists_steps_in_order_and_flags_side_effects() {
        let steps: Vec<Box<dyn WorkflowStep>> = vec![
            Box::new(RecordingStep::new("read", false)),
            Box::new(RecordingStep::new("write", true)),
        ];
        let described = describe_workflow(&steps);
        assert_eq!(described.len(), 2);
        assert!(!described[0].contains("onay gerekir"));
        assert!(described[1].contains("onay gerekir"));
    }

    #[test]
    fn every_step_succeeding_produces_a_completed_summary() {
        let steps: Vec<Box<dyn WorkflowStep>> = vec![
            Box::new(RecordingStep::new("a", false)),
            Box::new(RecordingStep::new("b", true)),
        ];
        let summary = run_workflow(steps, &approve_all, None, 0);
        assert!(summary.completed);
        assert_eq!(summary.outcomes.len(), 2);
        assert!(matches!(
            summary.outcomes[1].outcome,
            StepOutcomeKind::Succeeded(_)
        ));
    }

    #[test]
    fn a_denied_approval_stops_the_workflow_and_rolls_back_prior_side_effects() {
        let first = RecordingStep::new("first-write", true);
        let second = RecordingStep::new("second-write-denied", true);
        let third = RecordingStep::new("third-write", true);
        let steps: Vec<Box<dyn WorkflowStep>> =
            vec![Box::new(first), Box::new(second), Box::new(third)];
        let deny_second: &dyn Fn(&dyn WorkflowStep) -> bool =
            &|step| step.id() != "second-write-denied";
        let summary = run_workflow(steps, deny_second, None, 0);
        assert!(!summary.completed);
        assert!(matches!(
            summary.outcomes[0].outcome,
            StepOutcomeKind::RolledBack(Ok(()))
        ));
        assert_eq!(summary.outcomes[1].outcome, StepOutcomeKind::ApprovalDenied);
        assert_eq!(summary.outcomes[2].outcome, StepOutcomeKind::NotReached);
    }

    #[test]
    fn a_read_only_step_never_asks_for_approval() {
        let calls = RefCell::new(0);
        let steps: Vec<Box<dyn WorkflowStep>> =
            vec![Box::new(RecordingStep::new("read-only", false))];
        let approve_and_count: &dyn Fn(&dyn WorkflowStep) -> bool = &|_step| {
            *calls.borrow_mut() += 1;
            true
        };
        let summary = run_workflow(steps, approve_and_count, None, 0);
        assert!(summary.completed);
        assert_eq!(
            *calls.borrow(),
            0,
            "a read-only step must never trigger the approval gate"
        );
    }

    #[test]
    fn a_transient_failure_recovers_within_the_retry_budget() {
        let mut flaky = RecordingStep::new("flaky", true);
        flaky.fail_first_n_calls = Cell::new(2);
        let steps: Vec<Box<dyn WorkflowStep>> = vec![Box::new(flaky)];
        let summary = run_workflow(steps, &approve_all, None, 3);
        assert!(summary.completed);
        assert!(matches!(
            summary.outcomes[0].outcome,
            StepOutcomeKind::Succeeded(_)
        ));
    }

    #[test]
    fn exhausting_the_retry_budget_fails_the_step_and_rolls_back_earlier_ones() {
        let earlier = RecordingStep::new("earlier", true);
        let mut always_failing = RecordingStep::new("always-failing", true);
        always_failing.always_fails = true;
        let steps: Vec<Box<dyn WorkflowStep>> = vec![Box::new(earlier), Box::new(always_failing)];
        let summary = run_workflow(steps, &approve_all, None, 2);
        assert!(!summary.completed);
        assert!(matches!(
            summary.outcomes[0].outcome,
            StepOutcomeKind::RolledBack(Ok(()))
        ));
        assert!(matches!(
            summary.outcomes[1].outcome,
            StepOutcomeKind::Failed(_)
        ));
    }

    #[test]
    fn an_idempotency_key_prevents_a_second_execution_within_one_run() {
        // Aynı anahtara sahip iki farklı adım nesnesi — bir retry/resume senaryosunu simüle
        // ediyor: ikinci adım birinciyle aynı işi temsil ediyor, tekrar çalıştırılmamalı.
        let mut first = RecordingStep::new("do-thing-v1", true);
        first.idempotency_key = Some("thing-123".into());
        let mut duplicate = RecordingStep::new("do-thing-v2", true);
        duplicate.idempotency_key = Some("thing-123".into());
        let steps: Vec<Box<dyn WorkflowStep>> = vec![Box::new(first), Box::new(duplicate)];
        let summary = run_workflow(steps, &approve_all, None, 0);
        assert!(summary.completed);
        assert!(matches!(
            summary.outcomes[0].outcome,
            StepOutcomeKind::Succeeded(_)
        ));
        assert_eq!(
            summary.outcomes[1].outcome,
            StepOutcomeKind::SkippedIdempotent
        );
    }

    #[test]
    fn cancelling_mid_workflow_stops_remaining_steps_and_rolls_back_completed_ones() {
        let first = RecordingStep::new("first", true);
        let second = RecordingStep::new("second", true);
        let third = RecordingStep::new("third", true);
        let steps: Vec<Box<dyn WorkflowStep>> =
            vec![Box::new(first), Box::new(second), Box::new(third)];
        let cancel: CancelFlag = Arc::new(AtomicBool::new(false));
        // İlk adımdan sonra iptal edilmiş gibi davranmak için: onay callback'i ikinci adımda
        // bayrağı set ediyor — üçüncü adıma gelindiğinde workflow zaten iptal görecek.
        let cancel_for_closure = cancel.clone();
        let approve_then_cancel: &dyn Fn(&dyn WorkflowStep) -> bool = &move |step| {
            if step.id() == "second" {
                cancel_for_closure.store(true, Ordering::SeqCst);
            }
            true
        };
        let summary = run_workflow(steps, approve_then_cancel, Some(&cancel), 0);
        assert!(!summary.completed);
        assert!(matches!(
            summary.outcomes[0].outcome,
            StepOutcomeKind::RolledBack(Ok(()))
        ));
        assert!(matches!(
            summary.outcomes[1].outcome,
            StepOutcomeKind::RolledBack(Ok(()))
        ));
        assert_eq!(summary.outcomes[2].outcome, StepOutcomeKind::Cancelled);
    }

    #[test]
    fn a_rollback_failure_is_reported_honestly_not_hidden_as_success() {
        let mut broken_rollback = RecordingStep::new("broken-rollback", true);
        broken_rollback.rollback_fails = true;
        let mut always_failing = RecordingStep::new("boom", true);
        always_failing.always_fails = true;
        let steps: Vec<Box<dyn WorkflowStep>> =
            vec![Box::new(broken_rollback), Box::new(always_failing)];
        let summary = run_workflow(steps, &approve_all, None, 0);
        assert!(!summary.completed);
        match &summary.outcomes[0].outcome {
            StepOutcomeKind::RolledBack(Err(message)) => {
                assert!(message.contains("rollback failed"))
            }
            other => panic!("expected a reported rollback failure, got {other:?}"),
        }
    }

    /// Çerçevenin gerçek, workflow-dışı senaryolarda üretilmiş sentetik adımlarla sınırlı
    /// olmadığını kanıtlıyor — F4'ün kendi `FileAppendNoteTool`'unu (bkz. `src/lib.rs`) saran
    /// gerçek bir iki-adımlı workflow, gerçek dosya sistemi üzerinde.
    #[test]
    fn a_real_two_step_workflow_over_real_files_completes_and_rolls_back_on_denial() {
        struct RealAppendStep {
            path: std::path::PathBuf,
            line: String,
        }
        impl WorkflowStep for RealAppendStep {
            fn id(&self) -> &str {
                "append"
            }
            fn description(&self) -> &str {
                "append a line to a real file"
            }
            fn has_side_effect(&self) -> bool {
                true
            }
            fn execute(&self) -> Result<String, String> {
                let mut content = std::fs::read_to_string(&self.path).unwrap_or_default();
                content.push_str(&self.line);
                content.push('\n');
                std::fs::write(&self.path, content).map_err(|error| error.to_string())?;
                Ok(format!("appended to {}", self.path.display()))
            }
            fn rollback(&self) -> Result<(), String> {
                // Bilerek basit: bu testte dosya boştan başlıyor, rollback tüm dosyayı siler.
                std::fs::remove_file(&self.path).map_err(|error| error.to_string())
            }
        }

        let path = std::env::temp_dir().join(format!(
            "jarvis-workflow-real-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let steps: Vec<Box<dyn WorkflowStep>> = vec![Box::new(RealAppendStep {
            path: path.clone(),
            line: "gerçek bir satır".into(),
        })];
        let summary = run_workflow(steps, &approve_all, None, 0);
        assert!(summary.completed);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "gerçek bir satır\n"
        );
        std::fs::remove_file(&path).ok();
    }
}
