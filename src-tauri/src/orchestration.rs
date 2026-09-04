use serde::{Deserialize, Serialize};

pub const NORMAL_CALL_BUDGET: u8 = 5;
// 분류 1 + 전문 직원 35명 계획/보고 70 + 부서장 취합 8 + 본부장 종합 1.
pub const IMPORTANT_CALL_BUDGET: u8 = 80;
pub const MAX_CONCURRENCY: u8 = 2;
pub const USAGE_WARNING_PERCENT: u8 = 80;
pub const USAGE_STOP_PERCENT: u8 = 95;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgendaImportance {
    Normal,
    Important,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgendaExecutionPolicy {
    pub importance: AgendaImportance,
    pub call_budget: u8,
    pub max_concurrency: u8,
    pub usage_stop_percent: u8,
    pub usage_warning_percent: u8,
    pub can_start: bool,
    pub warning: Option<String>,
    pub message: String,
}

pub fn plan_agenda(
    importance: AgendaImportance,
    current_usage_percent: Option<f64>,
) -> AgendaExecutionPolicy {
    let call_budget = match importance {
        AgendaImportance::Normal => NORMAL_CALL_BUDGET,
        AgendaImportance::Important => IMPORTANT_CALL_BUDGET,
    };
    let can_start = current_usage_percent
        .is_some_and(|usage| usage.is_finite() && usage < f64::from(USAGE_STOP_PERCENT));
    AgendaExecutionPolicy {
        importance,
        call_budget,
        max_concurrency: MAX_CONCURRENCY,
        usage_stop_percent: USAGE_STOP_PERCENT,
        usage_warning_percent: USAGE_WARNING_PERCENT,
        can_start,
        warning: current_usage_percent.and_then(|usage| {
            (usage.is_finite()
                && usage >= f64::from(USAGE_WARNING_PERCENT)
                && usage < f64::from(USAGE_STOP_PERCENT))
            .then(|| {
                format!(
                    "Codex 사용량이 {USAGE_WARNING_PERCENT}% 이상입니다. 작업은 계속하지만 공급자 한도에 따라 중간에 멈출 수 있으며 {USAGE_STOP_PERCENT}%부터 새 호출을 차단합니다."
                )
            })
        }),
        message: if can_start {
            format!(
                "최대 {call_budget}회 호출 · 동시 {}명 · 사용량 {}% 경고 · {}%에서 중단",
                MAX_CONCURRENCY, USAGE_WARNING_PERCENT, USAGE_STOP_PERCENT
            )
        } else if current_usage_percent.is_none() {
            "Codex 사용량을 확인할 수 없어 새 안건을 시작하지 않습니다.".to_owned()
        } else {
            format!(
                "Codex 사용량이 안전 중단선 {}%에 도달해 새 안건을 시작하지 않습니다.",
                USAGE_STOP_PERCENT
            )
        },
    }
}

#[tauri::command]
pub fn agenda_execution_policy(
    importance: AgendaImportance,
    current_usage_percent: Option<f64>,
) -> AgendaExecutionPolicy {
    plan_agenda(importance, current_usage_percent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_the_approved_normal_and_important_budgets() {
        assert_eq!(
            plan_agenda(AgendaImportance::Normal, Some(10.0)).call_budget,
            5
        );
        assert_eq!(
            plan_agenda(AgendaImportance::Important, Some(10.0)).call_budget,
            80
        );
        assert_eq!(MAX_CONCURRENCY, 2);
    }

    #[test]
    fn blocks_new_agendas_at_or_above_the_usage_cutoff() {
        assert!(plan_agenda(AgendaImportance::Normal, Some(79.9)).can_start);
        let warning = plan_agenda(AgendaImportance::Normal, Some(80.0));
        assert!(warning.can_start);
        assert!(warning.warning.is_some());
        assert!(plan_agenda(AgendaImportance::Important, Some(94.9)).can_start);
        assert!(!plan_agenda(AgendaImportance::Normal, Some(95.0)).can_start);
        assert!(!plan_agenda(AgendaImportance::Normal, None).can_start);
        assert!(!plan_agenda(AgendaImportance::Important, Some(f64::NAN)).can_start);
    }
}
