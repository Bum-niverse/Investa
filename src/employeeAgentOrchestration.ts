export const EMPLOYEE_AGENT_V2_DEPARTMENT_IDS = new Set([
  "research", "strategy", "risk", "execution", "digital-assets", "public-relations", "engineering", "compliance",
]);
export const MEETING_AGENT_TURN_TIMEOUT_MS = 180_000;
export const MEETING_ROLE_REPORT_TIMEOUT_MS = 300_000;
export const MEETING_LONG_REPORT_TIMEOUT_MS = 300_000;
export const CRITICAL_OFFICIAL_EVIDENCE_CONFIDENCE_CAP = 35;

export const codexUsageResetMessage = (usage?: {
  usedPercent: number;
  resetsAtSeconds: number;
} | null, locale = "ko-KR") => {
  if (!usage) return "현재 사용량 또는 초기화 시각을 확인할 수 없습니다.";
  return `현재 ${usage.usedPercent.toFixed(0)}% · ${new Date(usage.resetsAtSeconds * 1_000).toLocaleString(locale)} 초기화 예정`;
};

type EvidenceForCalibration = string | { evidenceId: string; source?: string | null };

const VERIFIED_CORPORATE_ACTION_HOSTS = new Set([
  "dart.fss.or.kr",
  "opendart.fss.or.kr",
  "kind.krx.co.kr",
  "data.krx.co.kr",
  "hanwhacorp.co.kr",
  "www.hanwhacorp.co.kr",
]);

const evidenceIdOf = (evidence: EvidenceForCalibration) => typeof evidence === "string" ? evidence : evidence.evidenceId;

const isVerifiedCorporateActionSource = (source?: string | null) => {
  if (!source) return false;
  try {
    const url = new URL(source);
    return url.protocol === "https:" && VERIFIED_CORPORATE_ACTION_HOSTS.has(url.hostname.toLowerCase());
  } catch {
    return false;
  }
};

const CORPORATE_ACTION_TERMS = ["인적분할", "물적분할", "회사분할", "분할", "신주", "배정", "재상장", "거래정지", "합병"];

export const requiresCorporateActionOfficialEvidence = (topic: string) =>
  CORPORATE_ACTION_TERMS.some((term) => topic.includes(term));

export const corporateActionEvidenceCalibration = (
  topic: string,
  evidence: Iterable<EvidenceForCalibration>,
  reportedConfidencePercent: number,
) => {
  const required = requiresCorporateActionOfficialEvidence(topic);
  const hasOfficialEvidence = Array.from(evidence).some((item) =>
    evidenceIdOf(item).startsWith("opendart-corporate-action-")
      || typeof item !== "string" && isVerifiedCorporateActionSource(item.source));
  return {
    required,
    hasOfficialEvidence,
    confidencePercent: required && !hasOfficialEvidence
      ? Math.min(reportedConfidencePercent, CRITICAL_OFFICIAL_EVIDENCE_CONFIDENCE_CAP)
      : reportedConfidencePercent,
  };
};

export const detailedRoleFinding = (summary: string, findings: string[], maximumCharacters = 1_000) => {
  const sections = [summary.trim(), ...findings.map((finding) => finding.trim()).filter(Boolean)].filter(Boolean);
  return truncatePromptText(Array.from(new Set(sections)).join("\n\n"), maximumCharacters);
};

export type MeetingAgentTurnStage = "agenda_routing" | "tool_plan" | "role_report" | "department_report" | "meeting_synthesis";

export const meetingAgentTurnTimeoutMs = (stage: MeetingAgentTurnStage) =>
  stage === "role_report" || stage === "department_report" || stage === "meeting_synthesis"
    ? MEETING_LONG_REPORT_TIMEOUT_MS
    : MEETING_AGENT_TURN_TIMEOUT_MS;

export const boundedDepartmentEvidenceGap = (gaps: Iterable<string>, maximumCharacters = 500) => {
  const joined = Array.from(gaps, (gap) => gap.trim()).filter(Boolean).join(" · ");
  if (!joined) return null;
  return Array.from(joined).slice(0, maximumCharacters).join("");
};

type DepartmentRoleFindingInput = {
  agentId: string;
  role: string;
  finding: string;
  evidenceIds: string[];
  counterevidence: string[];
  evidenceGap?: string | null;
};

type AggregationEvidenceInput = {
  evidenceId: string;
  source: string;
  sourceRevision?: string | null;
  observation: string;
  observedAt?: string | null;
};

type DepartmentAgentIdentity = {
  id: string;
  name: string;
};

export const preserveEmployeeFindingsAfterManagerFailure = (
  department: { id: string; name: string; agents: DepartmentAgentIdentity[] },
  findings: Record<string, Omit<DepartmentRoleFindingInput, "agentId"> & { agentId?: string }>,
  message: string,
) => ({
  departmentId: department.id,
  departmentName: department.name,
  conclusion: "watch" as const,
  confidencePercent: 0,
  summary: `부서장 종합은 완료되지 않았습니다. 직원별 검증 결과와 근거는 손실 없이 보존하며, 부서 승인 전까지 판단은 보류합니다. ${message}`,
  roleFindings: department.agents.slice(1).map((agent) => compactDepartmentRoleFinding(
    findings[agent.id] ? { ...findings[agent.id], agentId: agent.id } : {
      agentId: agent.id,
      role: agent.name,
      finding: "업무를 완료하지 못했습니다.",
      evidenceIds: [],
      counterevidence: [],
      evidenceGap: message,
    },
  )),
  risks: ["부서장 종합·승인이 완료되지 않아 직원별 결과를 부서 결론으로 해석할 수 없습니다."],
  nextActions: ["보존된 직원 근거를 사용해 부서장 종합만 새 작업에서 다시 실행합니다."],
});

const truncatePromptText = (value: string, maximumCharacters: number) =>
  Array.from(value.trim()).slice(0, maximumCharacters).join("");

const normalizePromptEvidenceText = (value: string, maximumCharacters: number) =>
  truncatePromptText(value.replace(/[\u0000-\u001f\u007f]/g, " ").replace(/\s+/g, " "), maximumCharacters);

export const compactEvidenceForAggregation = (
  agentId: string,
  evidence: AggregationEvidenceInput[],
) => evidence.slice(0, 12).map((item) => ({
  agentId: normalizePromptEvidenceText(agentId, 80),
  evidenceId: normalizePromptEvidenceText(item.evidenceId, 120),
  source: normalizePromptEvidenceText(item.source, 500),
  sourceRevision: item.sourceRevision ? normalizePromptEvidenceText(item.sourceRevision, 120) : null,
  observation: normalizePromptEvidenceText(item.observation, 800),
  observedAt: item.observedAt ? normalizePromptEvidenceText(item.observedAt, 80) : null,
}));

export const compactDepartmentRoleFinding = (finding: DepartmentRoleFindingInput) => ({
  agentId: finding.agentId,
  role: truncatePromptText(finding.role, 80),
  finding: truncatePromptText(finding.finding, 1_000),
  evidenceIds: Array.from(new Set(finding.evidenceIds)).slice(0, 12),
  counterevidence: finding.counterevidence.slice(0, 6).map((item) => truncatePromptText(item, 500)),
  evidenceGap: finding.evidenceGap ? truncatePromptText(finding.evidenceGap, 500) : null,
});

export const allowedEvidenceBoundaryPrompt = (evidenceIds: Iterable<string>) =>
  `[이번 보고에서 허용된 evidenceId]\n${JSON.stringify(Array.from(new Set(evidenceIds)))}\n\nRoleReport.evidence[].evidenceId에는 위 배열의 문자열만 그대로 사용하세요. 배열이 비었거나 필요한 공식 근거가 없으면 evidence를 빈 배열로 두고 evidenceGaps와 nextRequests에 결측을 기록하세요. 설명용 이름, 임시 ID, mv-* 같은 새 ID를 만들지 마세요.`;

export const meetingAgentTimeoutMessage = (stage: MeetingAgentTurnStage) => ({
  agenda_routing: "안건 분류가 3분 안에 끝나지 않아 회의를 시작하지 않았습니다.",
  tool_plan: "읽기 전용 도구 선택이 3분 안에 끝나지 않아 해당 직원 업무를 근거 공백으로 처리했습니다.",
  role_report: "도구 결과 기반 역할 보고가 5분 안에 끝나지 않아 해당 직원 업무를 근거 공백으로 처리했습니다.",
  department_report: "부서 종합 보고가 5분 안에 끝나지 않아 해당 부서를 근거 공백으로 처리했습니다.",
  meeting_synthesis: "본부장 최종 종합이 5분 안에 끝나지 않아 결과를 보류했습니다.",
})[stage];

export type AgentToolPlan = {
  agentId: string;
  rationale: string;
  requests: Array<{ toolId: string; reason: string }>;
  canProceedWithoutTools: boolean;
  prohibitedActionsAcknowledged: boolean;
};

export const AGENT_TOOL_IDS: Readonly<Record<string, readonly string[]>> = {
  "technical-analyst": ["analysis.price_technical"],
  "fundamental-analyst": ["analysis.fundamentals_filings", "research.codex_web_search"],
  "news-analyst": ["analysis.telegram_news", "analysis.disclosure_news", "research.codex_web_search"],
  "macro-analyst": ["analysis.market_regime"],
  "paper-researcher": ["research.crossref_metadata", "research.github_repository", "research.codex_web_search"],
  "bull-researcher": ["analysis.price_technical", "analysis.fundamentals_filings", "analysis.disclosure_news", "analysis.telegram_news", "analysis.market_regime"],
  "bear-researcher": ["analysis.price_technical", "analysis.fundamentals_filings", "analysis.disclosure_news", "analysis.telegram_news", "analysis.market_regime"],
  trader: ["analysis.price_technical", "analysis.market_regime", "analysis.position_portfolio"],
  "strategy-researcher": ["analysis.price_technical", "analysis.market_regime"],
  "aggressive-risk": ["analysis.price_technical", "analysis.fundamentals_filings", "analysis.market_regime", "analysis.position_portfolio"],
  "neutral-risk": ["analysis.price_technical", "analysis.fundamentals_filings", "analysis.market_regime", "analysis.position_portfolio"],
  "conservative-risk": ["analysis.price_technical", "analysis.fundamentals_filings", "analysis.market_regime", "analysis.position_portfolio"],
  "risk-monitor": ["analysis.position_portfolio", "analysis.market_regime"],
  "model-validator": ["analysis.price_technical", "analysis.fundamentals_filings", "analysis.market_regime"],
  "broker-operator": ["analysis.price_technical", "analysis.position_portfolio", "operations.runtime_snapshot"],
  "ledger-operator": ["operations.paper_ledger_snapshot", "operations.audit_snapshot"],
  reconciliation: ["operations.runtime_snapshot", "operations.paper_ledger_snapshot"],
  "kill-switch": ["operations.runtime_snapshot", "operations.audit_snapshot"],
  "trade-quality": ["analysis.price_technical", "operations.paper_ledger_snapshot"],
  "spot-analyst": ["analysis.price_technical", "analysis.market_regime", "analysis.telegram_news"],
  derivatives: ["analysis.price_technical", "analysis.market_regime", "operations.runtime_snapshot"],
  onchain: ["analysis.telegram_news"],
  "crypto-ops": ["analysis.market_regime", "operations.runtime_snapshot"],
  writer: ["analysis.evidence_manifest", "operations.audit_snapshot"],
  "fact-editor": ["analysis.evidence_manifest", "operations.audit_snapshot", "operations.paper_ledger_snapshot"],
  "media-editor": ["analysis.evidence_manifest"],
  archivist: ["analysis.evidence_manifest", "operations.audit_snapshot"],
  "data-engineer": ["analysis.evidence_manifest", "analysis.price_technical", "analysis.market_regime"],
  "quant-engineer": ["analysis.price_technical", "analysis.evidence_manifest", "operations.runtime_snapshot"],
  mlops: ["analysis.evidence_manifest", "operations.runtime_snapshot", "operations.audit_snapshot"],
  sre: ["operations.runtime_snapshot", "operations.audit_snapshot", "analysis.evidence_manifest"],
  "algorithm-auditor": ["operations.audit_snapshot", "operations.runtime_snapshot", "analysis.evidence_manifest"],
  "restriction-officer": ["operations.runtime_snapshot", "analysis.position_portfolio"],
  "replay-officer": ["operations.audit_snapshot", "operations.paper_ledger_snapshot"],
  "publication-compliance": ["analysis.evidence_manifest", "operations.audit_snapshot"],
};

export const agentToolPlanMatchesFrontendCatalog = (plan: AgentToolPlan) => {
  const allowed = AGENT_TOOL_IDS[plan.agentId] ?? [];
  const selected = plan.requests.map((request) => request.toolId);
  return plan.rationale.trim().length > 0
    && plan.rationale.length <= 2_000
    && plan.prohibitedActionsAcknowledged
    && selected.length <= 3
    && (selected.length > 0 || plan.canProceedWithoutTools)
    && new Set(selected).size === selected.length
    && plan.requests.every((request) => request.reason.trim().length > 0 && request.reason.length <= 500)
    && selected.every((toolId) => allowed.includes(toolId));
};

const REQUIRED_AGENT_TOOL_IDS: Readonly<Record<string, readonly string[]>> = {
  "technical-analyst": ["analysis.price_technical"],
  "fundamental-analyst": ["analysis.fundamentals_filings"],
  "news-analyst": ["analysis.telegram_news", "analysis.disclosure_news"],
};

export const withRequiredAgentTools = (plan: AgentToolPlan): AgentToolPlan => {
  const required = REQUIRED_AGENT_TOOL_IDS[plan.agentId] ?? [];
  const requests = [...plan.requests];
  for (const toolId of required) {
    if (requests.some((request) => request.toolId === toolId)) continue;
    requests.unshift({ toolId, reason: "전체 분석의 필수 근거 가용성을 실행 전에 확인" });
  }
  return { ...plan, requests: requests.slice(0, 3), canProceedWithoutTools: false };
};

type AvailabilitySnapshot = {
  provider: string;
  symbol: string;
  completedBarCount: number;
  adjusted: boolean;
  indicators: Record<string, number | null | undefined>;
  annotationCount: number;
  annotationKinds: string[];
  fundamentalCount: number;
  filingCount: number;
};

type AvailabilityEvidence = { agentId: string; evidenceId: string; source: string };

export const buildDepartmentEvidenceAvailabilityManifest = (input: {
  snapshots: AvailabilitySnapshot[];
  evidence: AvailabilityEvidence[];
  telegramIncludedCount: number;
  telegramSyncStatus: string;
}) => {
  const uniqueEvidence = Array.from(new Map(input.evidence.map((item) => [item.evidenceId, item])).values());
  const providerContracts = Array.from(new Set(input.snapshots.map((snapshot) => snapshot.provider))).map((provider) => ({
    provider,
    requiredContract: provider === "TOSS_OPEN_API" ? "TOSS_OPEN_API" : provider.toUpperCase().includes("KIS") ? "KIS" : provider,
  }));
  return {
    schemaVersion: "1.0",
    authority: "deterministic_runtime_manifest",
    providerContracts,
    technical: input.snapshots.map((snapshot) => ({
      provider: snapshot.provider,
      symbol: snapshot.symbol,
      completedBarCount: snapshot.completedBarCount,
      adjusted: snapshot.adjusted,
      availableIndicators: Object.entries(snapshot.indicators).filter(([, value]) => Number.isFinite(value)).map(([key]) => key),
      annotationCount: snapshot.annotationCount,
      annotationKinds: Array.from(new Set(snapshot.annotationKinds)),
    })),
    fundamentals: { itemCount: input.snapshots.reduce((sum, snapshot) => sum + snapshot.fundamentalCount, 0) },
    filings: {
      itemCount: input.snapshots.reduce((sum, snapshot) => sum + snapshot.filingCount, 0),
      employeeEvidenceCount: uniqueEvidence.filter((item) => item.evidenceId.startsWith("opendart-") || item.evidenceId.includes("-filing-")).length,
    },
    generalNews: {
      employeeEvidenceCount: uniqueEvidence.filter((item) => item.agentId === "news-analyst" && (item.evidenceId.startsWith("naver-news-") || item.evidenceId.startsWith("codex-web-"))).length,
    },
    telegram: {
      includedCount: input.telegramIncludedCount,
      employeeEvidenceCount: uniqueEvidence.filter((item) => item.evidenceId.startsWith("telegram-")).length,
      syncStatus: input.telegramSyncStatus,
    },
    httpsSourceUrlCount: uniqueEvidence.filter((item) => item.source.startsWith("https://")).length,
    rule: "직원 서술은 이 매니페스트의 존재 여부를 뒤집을 수 없고, 실제 0건인 항목만 근거 공백으로 판정한다.",
  };
};

export const telegramToolEvidenceStatus = (itemCount: number, syncError?: string | null) =>
  itemCount <= 0 ? "unavailable" : syncError ? "cached_offline" : "completed";

export const sanitizePaperAccountsForAgent = (accounts: Array<{ account: {
  accountId?: string;
  currency: string;
  cashMinor: number;
  realizedPnlMinor: number;
  eventCount: number;
  lastEventAtMs: number;
  positions: Record<string, { symbol: string; quantity: number; quantityScale: number; costBasisMinor: number }>;
} }>) => accounts.map((item) => ({
  currency: item.account.currency,
  cashMinor: item.account.cashMinor,
  realizedPnlMinor: item.account.realizedPnlMinor,
  eventCount: item.account.eventCount,
  lastEventAtMs: item.account.lastEventAtMs,
  positions: Object.values(item.account.positions).map((position) => ({
    symbol: position.symbol,
    quantity: position.quantity,
    quantityScale: position.quantityScale,
    costBasisMinor: position.costBasisMinor,
  })),
}));

export const sanitizeAuditEventsForAgent = (events: Array<{
  action: string;
  occurredAtMs: number;
  actor?: string;
  targetId?: string;
  detail?: string;
  previousHash?: string;
  nextHash?: string;
  correlationId?: string;
}>) => events.slice(0, 50).map((item) => ({ action: item.action, occurredAtMs: item.occurredAtMs }));

export const EMPLOYEE_TASKS: Readonly<Record<string, string>> = {
  "technical-analyst": "제공된 가격·거래량·지표·차트 근거만 사용해 기술 구조와 반대 신호를 분석하세요.",
  "fundamental-analyst": "제공된 재무·공시 근거만 사용해 펀더멘털과 확인 불가능한 항목을 구분하세요.",
  "news-analyst": "제공된 공시·Telegram 뉴스 근거의 사실성·시점·중복·시장 반응 공백을 분석하세요.",
  "macro-analyst": "제공된 시장·업종·수급·거시 근거 범위에서 레짐과 공백을 분석하세요.",
  "paper-researcher": "안건과 관련된 공개 퀀트 논문을 검색해 서지 메타데이터 후보를 확보하고, 원문 검증 전 단계임을 구분하여 전략 가정·재현 조건·제품 적용 가능성을 검토하세요.",
  "bull-researcher": "제공된 근거에서 상승 촉매·상승 경로·성립 조건과 상승 논리 약화 요인만 독립적으로 분석하세요.",
  "bear-researcher": "제공된 근거에서 하락 위험·상승 논리의 취약점·하락 경로와 반박 조건만 독립적으로 분석하세요.",
  trader: "검증된 가격·시장·포지션 근거가 있는 범위에서만 진입·손절·목표·보유기간 초안을 만들고 수량이나 주문은 실행하지 마세요.",
  "strategy-researcher": "제공된 완료 봉과 시장 레짐으로 백테스트·walk-forward에 필요한 가정·비용·분할 조건을 검토하고 성과를 꾸며내지 마세요.",
  "aggressive-risk": "사용자가 설정한 한도를 완화하지 말고 허용 범위 안의 적극적 대안과 손실 경로를 검토하세요.",
  "neutral-risk": "포지션·시장·재무 근거에서 기대값·변동성·상관관계의 균형 위험을 검토하세요.",
  "conservative-risk": "급변동·유동성 부족·손실 확대의 최악 경로와 방어 조건을 우선 검토하세요.",
  "risk-monitor": "저장된 포지션·현금·시장 상태만 사용해 손실·낙폭·노출 한도에 필요한 관측값과 근거 공백을 구분하세요.",
  "model-validator": "가격·재무·시장 근거의 시점 누수·과적합·표본·재현성과 확률 보정 위험을 독립적으로 검토하세요.",
  "broker-operator": "마스킹된 시장·포지션·운영 상태만 사용해 공급자 계약과 읽기 전용 연결 공백을 점검하고 주문을 호출하지 마세요.",
  "ledger-operator": "내부 모의원장 요약과 감사 사건만 사용해 상태 전이·중복·불변성 공백을 점검하고 원장을 수정하지 마세요.",
  reconciliation: "재시작 대사 상태와 내부 모의원장 요약을 비교해 불일치와 복구 제안만 작성하고 자동 정정하지 마세요.",
  "kill-switch": "관측된 운영·감사 상태에서 중지 필요 조건만 검토하고 실제 킬 스위치·취소·청산을 실행하지 마세요.",
  "trade-quality": "제공된 가격과 내부 모의원장 범위에서 체결 품질을 검토하고 없는 예상가·슬리피지를 추정하지 마세요.",
  "spot-analyst": "코인 현물 스냅샷과 저장 뉴스만 사용해 유동성·추세·반대 신호를 분석하고 파생 판단을 대신하지 마세요.",
  derivatives: "코인·증권 파생 스냅샷에서 확인된 마크·지수·펀딩·시장 상태만 분석하고 레버리지나 주문을 변경하지 마세요.",
  onchain: "선택 Telegram 근거 중 출처·관측 시각이 확인된 온체인 자료만 보조 근거로 검토하고 미제공 온체인 지표를 만들지 마세요.",
  "crypto-ops": "24시간 시장·운영 상태의 지연과 결측만 점검하고 거래소 연결·이체·주문을 실행하지 마세요.",
  writer: "근거 manifest와 감사 시각으로 확인된 사실만 시간순 개발기 초안으로 정리하고 외부 게시하지 마세요.",
  "fact-editor": "근거 manifest·감사·모의원장 수치의 출처 일치 여부만 검수하고 수익이나 완료 상태를 과장하지 마세요.",
  "media-editor": "제공된 화면·이미지 근거 존재 여부와 캡션·대체텍스트·모바일 검수 요구만 정리하고 파일을 수정하지 마세요.",
  archivist: "근거 manifest와 감사 사건을 공개 가능·내부 전용·결측으로 분류하고 비밀정보를 복사하거나 파일을 이동하지 마세요.",
  "data-engineer": "제공된 데이터의 시점·출처·결측·수정주가·라이선스 계약을 검토하고 값을 생성하지 마세요.",
  "quant-engineer": "가격·근거 계약과 운영 경계에서 지표·피처·백테스트 계산의 재현 조건과 테스트 공백을 검토하세요.",
  mlops: "근거 manifest·운영·감사 상태에서 전략 버전·승격·만료·드리프트의 필요한 게이트만 제안하고 배포하지 마세요.",
  sre: "운영·감사·근거 상태에서 가용성·비용·비밀정보·복구 위험을 검토하고 시스템 명령을 실행하지 마세요.",
  "algorithm-auditor": "감사·운영·근거 manifest로 전략·위험 게이트 변경의 추적성과 롤백 공백을 독립 검토하세요.",
  "restriction-officer": "운영 경계와 익명화 포지션만 사용해 주문 잠금·권한·시장 제한 위반 가능성을 점검하고 상태를 변경하지 마세요.",
  "replay-officer": "감사 사건과 내부 모의원장 요약으로 재현 가능한 순서와 누락만 조사하고 로그·원장을 수정하지 마세요.",
  "publication-compliance": "근거 manifest와 감사 기록으로 수익 표현·투자 오인·데이터·이미지 출처 공백을 검수하고 게시하지 마세요.",
};

export const usesEmployeeAgentV2 = (departmentId: string) => EMPLOYEE_AGENT_V2_DEPARTMENT_IDS.has(departmentId);

export const prioritizeEmployeeAgentDepartments = (departmentIds: string[]) => [...departmentIds]
  .sort((left, right) => Number(usesEmployeeAgentV2(right)) - Number(usesEmployeeAgentV2(left)));

const DEPARTMENT_EMPLOYEE_COUNTS: Readonly<Record<string, number>> = {
  research: 5,
  strategy: 4,
  risk: 5,
  execution: 5,
  "digital-assets": 4,
  "public-relations": 4,
  engineering: 4,
  compliance: 4,
};

export const departmentAnalysisCallCost = (departmentId: string) => {
  const employeeCount = DEPARTMENT_EMPLOYEE_COUNTS[departmentId];
  return employeeCount === undefined ? 1 : employeeCount * 2 + 1;
};

export type DepartmentCallBudgetPlan = {
  departmentIds: string[];
  employeeAgentDepartmentIds: string[];
  totalCallCount: number;
};

export const planDepartmentsWithinCallBudget = (departmentIds: string[], callBudget: number): DepartmentCallBudgetPlan => {
  const uniqueDepartmentIds = Array.from(new Set(departmentIds));
  let remaining = Math.max(0, callBudget - 2); // 안건 분류 + 본부장 종합
  const selectedDepartmentIds: string[] = [];
  const employeeAgentDepartmentIds: string[] = [];
  for (const departmentId of uniqueDepartmentIds) {
    const callCost = departmentAnalysisCallCost(departmentId);
    if (callCost > remaining) continue;
    selectedDepartmentIds.push(departmentId);
    if (usesEmployeeAgentV2(departmentId)) employeeAgentDepartmentIds.push(departmentId);
    remaining -= callCost;
  }
  return {
    departmentIds: selectedDepartmentIds,
    employeeAgentDepartmentIds,
    totalCallCount: 2 + selectedDepartmentIds.reduce((total, departmentId) => total + departmentAnalysisCallCost(departmentId), 0),
  };
};

export const selectDepartmentsWithinCallBudget = (departmentIds: string[], callBudget: number) =>
  planDepartmentsWithinCallBudget(departmentIds, callBudget).departmentIds;

export const allowedBrokerEvidenceIds = (agentId: string) => {
  const webEnabled = AGENT_TOOL_IDS[agentId]?.includes("research.codex_web_search") ?? false;
  return agentId === "paper-researcher"
    ? [
      ...Array.from({ length: 5 }, (_item, index) => `crossref-paper-${index + 1}`),
      ...Array.from({ length: 2 }, (_item, index) => `github-repository-${index + 1}`),
      ...Array.from({ length: 10 }, (_item, index) => `codex-web-${index + 1}`),
    ]
    : webEnabled ? Array.from({ length: 10 }, (_item, index) => `codex-web-${index + 1}`) : [];
};

export const brokerEvidenceIdsForTool = (toolId: string) => toolId === "research.crossref_metadata"
  ? Array.from({ length: 5 }, (_item, index) => `crossref-paper-${index + 1}`)
  : toolId === "research.github_repository"
    ? Array.from({ length: 2 }, (_item, index) => `github-repository-${index + 1}`)
    : toolId === "research.codex_web_search"
      ? Array.from({ length: 10 }, (_item, index) => `codex-web-${index + 1}`)
    : [];

export const codexWebRoleEvidenceIsValid = (evidence: { evidenceId: string; source: string; observedAt?: string | null }) => {
  if (!evidence.evidenceId.startsWith("codex-web-")) return true;
  return /^codex-web-(?:[1-9]|10)$/.test(evidence.evidenceId)
    && evidence.source.startsWith("https://")
    && Boolean(evidence.observedAt?.trim());
};

type ActiveMeetingDelegation = {
  managerId: string;
  status: "working" | "synthesizing" | string;
  activeAgentIds?: string[];
};

export const activeMeetingCodexAgentIds = (
  activeManagerIds: Iterable<string>,
  delegations: ActiveMeetingDelegation[],
) => Array.from(new Set([
  ...activeManagerIds,
  ...delegations.flatMap((delegation) => delegation.activeAgentIds ?? []),
  ...delegations.filter((delegation) => delegation.status === "synthesizing").map((delegation) => delegation.managerId),
]));
