import assert from "node:assert/strict";
import test from "node:test";
import { activeMeetingCodexAgentIds, agentToolPlanMatchesFrontendCatalog, allowedBrokerEvidenceIds, allowedEvidenceBoundaryPrompt, boundedDepartmentEvidenceGap, brokerEvidenceIdsForTool, buildDepartmentEvidenceAvailabilityManifest, codexUsageResetMessage, codexWebRoleEvidenceIsValid, compactDepartmentRoleFinding, compactEvidenceForAggregation, corporateActionEvidenceCalibration, CRITICAL_OFFICIAL_EVIDENCE_CONFIDENCE_CAP, departmentAnalysisCallCost, detailedRoleFinding, EMPLOYEE_TASKS, MEETING_AGENT_TURN_TIMEOUT_MS, MEETING_LONG_REPORT_TIMEOUT_MS, MEETING_ROLE_REPORT_TIMEOUT_MS, meetingAgentTimeoutMessage, meetingAgentTurnTimeoutMs, planDepartmentsWithinCallBudget, preserveEmployeeFindingsAfterManagerFailure, prioritizeEmployeeAgentDepartments, sanitizeAuditEventsForAgent, sanitizePaperAccountsForAgent, selectDepartmentsWithinCallBudget, telegramToolEvidenceStatus, usesEmployeeAgentV2, withRequiredAgentTools } from "../src/employeeAgentOrchestration.ts";

test("employee_agent_v2는 본부를 제외한 8개 전문 부서를 전환 대상으로 등록한다", () => {
  for (const departmentId of ["research", "strategy", "risk", "execution", "digital-assets", "public-relations", "engineering", "compliance"]) {
    assert.equal(usesEmployeeAgentV2(departmentId), true, departmentId);
  }
  assert.equal(usesEmployeeAgentV2("headquarters"), false);
});

test("브로커 근거 ID는 논문 연구원의 고정 메타데이터·웹 범위만 연다", () => {
  assert.deepEqual(allowedBrokerEvidenceIds("paper-researcher"), [
    "crossref-paper-1", "crossref-paper-2", "crossref-paper-3", "crossref-paper-4", "crossref-paper-5",
    "github-repository-1", "github-repository-2",
    ...Array.from({ length: 10 }, (_item, index) => `codex-web-${index + 1}`),
  ]);
  assert.deepEqual(allowedBrokerEvidenceIds("fundamental-analyst"), Array.from({ length: 10 }, (_item, index) => `codex-web-${index + 1}`));
  assert.deepEqual(allowedBrokerEvidenceIds("news-analyst"), Array.from({ length: 10 }, (_item, index) => `codex-web-${index + 1}`));
  assert.deepEqual(allowedBrokerEvidenceIds("technical-analyst"), []);
});

test("외부 브로커 근거 ID는 실제 선택한 도구 종류로 제한한다", () => {
  assert.deepEqual(brokerEvidenceIdsForTool("research.github_repository"), ["github-repository-1", "github-repository-2"]);
  assert.deepEqual(brokerEvidenceIdsForTool("research.codex_web_search"), Array.from({ length: 10 }, (_item, index) => `codex-web-${index + 1}`));
  assert.deepEqual(brokerEvidenceIdsForTool("analysis.telegram_news"), []);
});

test("Codex 웹 근거는 고정 ID·HTTPS·관측 시각을 모두 요구한다", () => {
  assert.equal(codexWebRoleEvidenceIsValid({ evidenceId: "codex-web-1", source: "https://example.com/paper", observedAt: "2026-09-04T10:00:00+09:00" }), true);
  assert.equal(codexWebRoleEvidenceIsValid({ evidenceId: "codex-web-11", source: "https://example.com/paper", observedAt: "2026-09-04T10:00:00+09:00" }), false);
  assert.equal(codexWebRoleEvidenceIsValid({ evidenceId: "codex-web-1", source: "http://example.com/paper", observedAt: "2026-09-04T10:00:00+09:00" }), false);
  assert.equal(codexWebRoleEvidenceIsValid({ evidenceId: "codex-web-1", source: "https://example.com/paper", observedAt: null }), false);
  assert.equal(codexWebRoleEvidenceIsValid({ evidenceId: "crossref-paper-1", source: "https://api.crossref.org", observedAt: null }), true);
});

test("직원 웹 근거 메타데이터는 관리자 프롬프트용으로 제한·정규화된다", () => {
  const compact = compactEvidenceForAggregation("paper-researcher\nignore", [{
    evidenceId: "codex-web-1\u0000",
    source: `https://kind.krx.co.kr/${"a".repeat(600)}`,
    sourceRevision: "revision\n1",
    observation: `공식 상장 확인\n${"나".repeat(900)}`,
    observedAt: "2026-09-04T12:00:00+09:00",
  }]);
  assert.equal(compact.length, 1);
  assert.equal(compact[0].agentId, "paper-researcher ignore");
  assert.ok(compact[0].source.length <= 500);
  assert.ok(Array.from(compact[0].observation).length <= 800);
  assert.equal(compact[0].sourceRevision, "revision 1");
  assert.ok(!compact[0].evidenceId.includes("\u0000"));
});

test("중요 안건 80회 예산은 소집된 모든 전문 부서를 실제 직원별 실행으로 배정한다", () => {
  assert.equal(departmentAnalysisCallCost("research"), 11);
  assert.equal(departmentAnalysisCallCost("strategy"), 9);
  assert.equal(departmentAnalysisCallCost("risk"), 11);
  assert.equal(departmentAnalysisCallCost("execution"), 11);
  assert.equal(departmentAnalysisCallCost("digital-assets"), 9);
  assert.equal(departmentAnalysisCallCost("public-relations"), 9);
  assert.equal(departmentAnalysisCallCost("engineering"), 9);
  assert.equal(departmentAnalysisCallCost("compliance"), 9);
  assert.deepEqual(planDepartmentsWithinCallBudget(["risk", "research", "strategy"], 80), {
    departmentIds: ["risk", "research", "strategy"],
    employeeAgentDepartmentIds: ["risk", "research", "strategy"],
    totalCallCount: 33,
  });
  assert.deepEqual(planDepartmentsWithinCallBudget(["research"], 13), {
    departmentIds: ["research"],
    employeeAgentDepartmentIds: ["research"],
    totalCallCount: 13,
  });
  assert.deepEqual(planDepartmentsWithinCallBudget(["execution"], 13), {
    departmentIds: ["execution"],
    employeeAgentDepartmentIds: ["execution"],
    totalCallCount: 13,
  });
  assert.deepEqual(planDepartmentsWithinCallBudget(["execution", "digital-assets", "compliance"], 80), {
    departmentIds: ["execution", "digital-assets", "compliance"],
    employeeAgentDepartmentIds: ["execution", "digital-assets", "compliance"],
    totalCallCount: 31,
  });
  assert.deepEqual(selectDepartmentsWithinCallBudget(["risk", "execution", "compliance"], 80), ["risk", "execution", "compliance"]);
  assert.deepEqual(planDepartmentsWithinCallBudget(["risk", "research"], 13), {
    departmentIds: ["risk"],
    employeeAgentDepartmentIds: ["risk"],
    totalCallCount: 13,
  });
});

test("기업행위 핵심 공식 근거가 없으면 근거 충족도를 결정론적으로 제한한다", () => {
  assert.deepEqual(corporateActionEvidenceCalibration("한화 인적분할 신주 배정", [], 98), {
    required: true,
    hasOfficialEvidence: false,
    confidencePercent: CRITICAL_OFFICIAL_EVIDENCE_CONFIDENCE_CAP,
  });
  assert.deepEqual(corporateActionEvidenceCalibration("한화 인적분할 신주 배정", ["opendart-corporate-action-20260904001234"], 88), {
    required: true,
    hasOfficialEvidence: true,
    confidencePercent: 88,
  });
  assert.deepEqual(corporateActionEvidenceCalibration("한화 인적분할 신주 배정", [{ evidenceId: "codex-web-1", source: "https://www.hanwhacorp.co.kr/hanwha/customer/notice_more.do?seq=1873" }], 82), {
    required: true,
    hasOfficialEvidence: true,
    confidencePercent: 82,
  });
  assert.equal(corporateActionEvidenceCalibration("한화 인적분할 신주 배정", [{ evidenceId: "codex-web-1", source: "https://evil.example/hanwha" }], 92).confidencePercent, CRITICAL_OFFICIAL_EVIDENCE_CONFIDENCE_CAP);
  assert.equal(corporateActionEvidenceCalibration("한화 인적분할 신주 배정", [{ evidenceId: "codex-web-1", source: "http://www.hanwhacorp.co.kr/notice" }], 92).confidencePercent, CRITICAL_OFFICIAL_EVIDENCE_CONFIDENCE_CAP);
  assert.equal(corporateActionEvidenceCalibration("한화 이동평균 분석", [], 91).confidencePercent, 91);
});

test("직원의 요약과 서로 다른 세부 결과를 부서장에게 함께 전달한다", () => {
  const finding = detailedRoleFinding("핵심 결론입니다.", ["핵심 결론입니다.", "공식 공시를 확인했습니다.", "반대 신호도 남아 있습니다."]);
  assert.equal(finding, "핵심 결론입니다.\n\n공식 공시를 확인했습니다.\n\n반대 신호도 남아 있습니다.");
  assert.ok(Array.from(detailedRoleFinding("요약", ["세부".repeat(800)])).length <= 1_000);
});

test("직원 보고 프롬프트는 전달된 근거 ID만 허용하고 임시 ID 생성을 금지한다", () => {
  const prompt = allowedEvidenceBoundaryPrompt(["official-1", "official-1", "market-2"]);
  assert.match(prompt, /\["official-1","market-2"\]/);
  assert.match(prompt, /위 배열의 문자열만 그대로/);
  assert.match(prompt, /mv-\*/);
});

test("부서장에게 전달하는 직원 근거 공백은 계약 상한에서 결정론적으로 잘린다", () => {
  assert.equal(boundedDepartmentEvidenceGap([]), null);
  assert.equal(boundedDepartmentEvidenceGap([" 첫째 ", "둘째"]), "첫째 · 둘째");
  assert.equal(Array.from(boundedDepartmentEvidenceGap(["가".repeat(600)]) ?? "").length, 500);
});

test("부서장은 상한이 적용된 직원별 결과만 전달받는다", () => {
  const compact = compactDepartmentRoleFinding({
    agentId: "risk-monitor",
    role: "한도·노출 모니터".repeat(20),
    finding: "판단".repeat(700),
    evidenceIds: [...Array.from({ length: 30 }, (_, index) => `evidence-${index}`), "evidence-0"],
    counterevidence: Array.from({ length: 8 }, () => "반대근거".repeat(200)),
    evidenceGap: "공백".repeat(400),
  });
  assert.equal(Array.from(compact.role).length, 80);
  assert.equal(Array.from(compact.finding).length, 1_000);
  assert.equal(compact.evidenceIds.length, 12);
  assert.equal(compact.counterevidence.length, 6);
  assert.equal(Array.from(compact.counterevidence[0]).length, 500);
  assert.equal(Array.from(compact.evidenceGap ?? "").length, 500);
});

test("전환 부서 직원은 각 역할에 한정된 독립 업무를 갖는다", () => {
  assert.equal(Object.keys(EMPLOYEE_TASKS).length, 35);
  for (const agentId of ["broker-operator", "ledger-operator", "spot-analyst", "onchain", "writer", "media-editor", "data-engineer", "sre", "algorithm-auditor", "publication-compliance"]) {
    assert.equal(typeof EMPLOYEE_TASKS[agentId], "string", agentId);
  }
  assert.match(EMPLOYEE_TASKS["paper-researcher"], /원문 검증 전 단계/);
  assert.match(EMPLOYEE_TASKS["bull-researcher"], /상승 촉매/);
  assert.match(EMPLOYEE_TASKS["risk-monitor"], /포지션·현금/);
});

test("재개 큐는 직원별 Agent 부서를 기존 라우팅 순서로 유지한다", () => {
  assert.deepEqual(prioritizeEmployeeAgentDepartments(["execution", "risk", "unknown", "research"]), ["execution", "risk", "research", "unknown"]);
});

test("회의 종료는 부장뿐 아니라 실행 중 직원과 종합 중 부장을 모두 취소한다", () => {
  assert.deepEqual(activeMeetingCodexAgentIds(new Set(["risk-director"]), [
    { managerId: "research-director", status: "working", activeAgentIds: ["technical-analyst", "news-analyst"] },
    { managerId: "strategy-director", status: "synthesizing", activeAgentIds: [] },
  ]), ["risk-director", "technical-analyst", "news-analyst", "strategy-director"]);
});

test("회의 Agent가 멈추면 단계별 제한시간 뒤 안전하게 종료한다", () => {
  assert.equal(MEETING_AGENT_TURN_TIMEOUT_MS, 180_000);
  assert.equal(MEETING_ROLE_REPORT_TIMEOUT_MS, 300_000);
  assert.equal(MEETING_LONG_REPORT_TIMEOUT_MS, 300_000);
  assert.equal(meetingAgentTurnTimeoutMs("tool_plan"), 180_000);
  assert.equal(meetingAgentTurnTimeoutMs("role_report"), 300_000);
  assert.equal(meetingAgentTurnTimeoutMs("department_report"), 300_000);
  assert.equal(meetingAgentTurnTimeoutMs("meeting_synthesis"), 300_000);
  assert.match(meetingAgentTimeoutMessage("agenda_routing"), /안건 분류.*3분.*시작하지 않았/);
  assert.match(meetingAgentTimeoutMessage("tool_plan"), /도구 선택.*3분.*근거 공백/);
  assert.match(meetingAgentTimeoutMessage("role_report"), /역할 보고.*5분.*근거 공백/);
  assert.match(meetingAgentTimeoutMessage("department_report"), /부서 종합.*5분.*근거 공백/);
  assert.match(meetingAgentTimeoutMessage("meeting_synthesis"), /본부장 최종 종합.*5분.*보류/);
});

test("부서장 종합 실패 시 직원 근거를 보존하되 부서 승인은 보류한다", () => {
  const report = preserveEmployeeFindingsAfterManagerFailure({
    id: "research",
    name: "리서치부",
    agents: [
      { id: "research-director", name: "리서치부장" },
      { id: "news-analyst", name: "뉴스 심리 분석가" },
      { id: "paper-researcher", name: "퀀트 논문 연구원" },
    ],
  }, {
    "paper-researcher": {
      role: "퀀트 논문 연구원",
      finding: "공식 회사분할 공시와 재상장 일정을 확인했습니다.",
      evidenceIds: ["codex-web-1", "codex-web-2"],
      counterevidence: ["단기 가격 효과는 확정할 수 없습니다."],
      evidenceGap: null,
    },
  }, "부서 종합 제한시간 초과");

  assert.equal(report.conclusion, "watch");
  assert.equal(report.confidencePercent, 0);
  assert.deepEqual(report.roleFindings[1].evidenceIds, ["codex-web-1", "codex-web-2"]);
  assert.equal(report.roleFindings[0].agentId, "news-analyst");
  assert.match(report.roleFindings[0].evidenceGap ?? "", /제한시간 초과/);
  assert.match(report.summary, /직원별 검증 결과와 근거는 손실 없이 보존/);
});

test("프런트 도구 실행도 Rust와 같은 역할별 허용 목록으로 이중 확인한다", () => {
  assert.equal(agentToolPlanMatchesFrontendCatalog({
    agentId: "paper-researcher",
    rationale: "공개 원문과 공식 문서를 직접 확인",
    requests: [{ toolId: "research.codex_web_search", reason: "메타데이터만으로 확인할 수 없는 원문 근거 조사" }],
    canProceedWithoutTools: false,
    prohibitedActionsAcknowledged: true,
  }), true);
  assert.equal(agentToolPlanMatchesFrontendCatalog({
    agentId: "news-analyst",
    rationale: "공식 뉴스 원문을 교차 확인",
    requests: [{ toolId: "research.codex_web_search", reason: "내부 공급자 공백의 공식 원문 조사" }],
    canProceedWithoutTools: false,
    prohibitedActionsAcknowledged: true,
  }), true);
  assert.equal(agentToolPlanMatchesFrontendCatalog({
    agentId: "risk-monitor",
    rationale: "보유 위험 확인",
    requests: [{ toolId: "analysis.position_portfolio", reason: "노출 근거 확인" }],
    canProceedWithoutTools: false,
    prohibitedActionsAcknowledged: true,
  }), true);
  assert.equal(agentToolPlanMatchesFrontendCatalog({
    agentId: "bull-researcher",
    rationale: "역할 초과 포지션 조회",
    requests: [{ toolId: "analysis.position_portfolio", reason: "계좌 확인" }],
    canProceedWithoutTools: false,
    prohibitedActionsAcknowledged: true,
  }), false);
  assert.equal(agentToolPlanMatchesFrontendCatalog({
    agentId: "ledger-operator",
    rationale: "내부 모의원장 상태 확인",
    requests: [{ toolId: "operations.paper_ledger_snapshot", reason: "불변 원장 요약 확인" }],
    canProceedWithoutTools: false,
    prohibitedActionsAcknowledged: true,
  }), true);
  assert.equal(agentToolPlanMatchesFrontendCatalog({
    agentId: "media-editor",
    rationale: "역할을 넘는 감사 상세 조회",
    requests: [{ toolId: "operations.audit_snapshot", reason: "감사 로그 확인" }],
    canProceedWithoutTools: false,
    prohibitedActionsAcknowledged: true,
  }), false);
  assert.equal(agentToolPlanMatchesFrontendCatalog({
    agentId: "news-analyst",
    rationale: "저장 뉴스 확인",
    requests: [{ toolId: "analysis.telegram_news", reason: "선택 채널 확인" }],
    canProceedWithoutTools: false,
    prohibitedActionsAcknowledged: true,
  }), true);
  assert.equal(agentToolPlanMatchesFrontendCatalog({
    agentId: "technical-analyst",
    rationale: "역할 초과 요청",
    requests: [{ toolId: "analysis.telegram_news", reason: "뉴스 확인" }],
    canProceedWithoutTools: false,
    prohibitedActionsAcknowledged: true,
  }), false);
  assert.equal(agentToolPlanMatchesFrontendCatalog({
    agentId: "news-analyst",
    rationale: "도구 없이 진행",
    requests: [],
    canProceedWithoutTools: false,
    prohibitedActionsAcknowledged: true,
  }), false);
});

test("전체 분석 담당자의 필수 근거 도구는 Agent 선택 누락과 무관하게 자동 보완한다", () => {
  const newsPlan = withRequiredAgentTools({
    agentId: "news-analyst",
    rationale: "웹 원문 조사",
    requests: [{ toolId: "research.codex_web_search", reason: "원문 확인" }],
    canProceedWithoutTools: false,
    prohibitedActionsAcknowledged: true,
  });
  assert.deepEqual(newsPlan.requests.map((request) => request.toolId), [
    "analysis.disclosure_news", "analysis.telegram_news", "research.codex_web_search",
  ]);
  assert.equal(agentToolPlanMatchesFrontendCatalog(newsPlan), true);

  const technicalPlan = withRequiredAgentTools({
    agentId: "technical-analyst",
    rationale: "도구 없이 판단",
    requests: [],
    canProceedWithoutTools: true,
    prohibitedActionsAcknowledged: true,
  });
  assert.deepEqual(technicalPlan.requests.map((request) => request.toolId), ["analysis.price_technical"]);
  assert.equal(technicalPlan.canProceedWithoutTools, false);
});

test("회의 차단 안내는 현재 Codex 사용량과 초기화 시각을 함께 표시한다", () => {
  const message = codexUsageResetMessage({ usedPercent: 84.6, resetsAtSeconds: 0 }, "en-US");
  assert.match(message, /현재 85%/);
  assert.match(message, /초기화 예정/);
  assert.equal(codexUsageResetMessage(null), "현재 사용량 또는 초기화 시각을 확인할 수 없습니다.");
});

test("결정론적 가용성 매니페스트는 TOSS 자료에 KIS 계약을 요구하거나 존재 근거를 결측 처리하지 않는다", () => {
  const manifest = buildDepartmentEvidenceAvailabilityManifest({
    snapshots: [{
      provider: "TOSS_OPEN_API",
      symbol: "000880",
      completedBarCount: 120,
      adjusted: true,
      indicators: { sma20: 111076.2, macdLine: 2400.1, bollingerUpper: 138000, missing: null },
      annotationCount: 4,
      annotationKinds: ["horizontal_line", "horizontal_line", "trend_line", "rectangle"],
      fundamentalCount: 0,
      filingCount: 0,
    }],
    evidence: [
      { agentId: "news-analyst", evidenceId: "telegram-1-1000-1", source: "https://t.me/example/1" },
      { agentId: "news-analyst", evidenceId: "codex-web-1", source: "https://example.com/news" },
      { agentId: "technical-analyst", evidenceId: "toss-000880-price", source: "TOSS_OPEN_API" },
    ],
    telegramIncludedCount: 2,
    telegramSyncStatus: "동기화 완료",
  });
  assert.deepEqual(manifest.providerContracts, [{ provider: "TOSS_OPEN_API", requiredContract: "TOSS_OPEN_API" }]);
  assert.deepEqual(manifest.technical[0].availableIndicators, ["sma20", "macdLine", "bollingerUpper"]);
  assert.deepEqual(manifest.technical[0].annotationKinds, ["horizontal_line", "trend_line", "rectangle"]);
  assert.equal(manifest.generalNews.employeeEvidenceCount, 1);
  assert.equal(manifest.telegram.employeeEvidenceCount, 1);
  assert.equal(manifest.httpsSourceUrlCount, 2);
});

test("Telegram 동기화 실패는 저장 근거가 있을 때만 오프라인 캐시로 판정한다", () => {
  assert.equal(telegramToolEvidenceStatus(3, "네트워크 연결 실패"), "cached_offline");
  assert.equal(telegramToolEvidenceStatus(0, "네트워크 연결 실패"), "unavailable");
  assert.equal(telegramToolEvidenceStatus(2, null), "completed");
});

test("직원용 운영 근거는 계좌 ID와 감사 상세 원문을 제거한다", () => {
  const accounts = sanitizePaperAccountsForAgent([{ account: {
    accountId: "sensitive-account-id", currency: "KRW", cashMinor: 1000, realizedPnlMinor: 10, eventCount: 2, lastEventAtMs: 3,
    positions: { A: { symbol: "005930", quantity: 1, quantityScale: 0, costBasisMinor: 500 } },
  } }]);
  assert.equal("accountId" in accounts[0], false);
  assert.equal(JSON.stringify(accounts).includes("sensitive-account-id"), false);
  const events = sanitizeAuditEventsForAgent([{ action: "paper.fill", occurredAtMs: 4, actor: "owner", targetId: "account-1", detail: "secret-like detail", previousHash: "a", nextHash: "b", correlationId: "c" }]);
  assert.deepEqual(events, [{ action: "paper.fill", occurredAtMs: 4 }]);
});
