export type MeetingHoldingItem = {
  symbol: string;
  name: string;
  marketCountry: string;
  currency: string;
  quantity: string;
  lastPrice: string;
  averagePurchasePrice: string;
};

export type MeetingAccountSnapshot = {
  provider: string;
  fetchedAtMs: number;
  readOnly: boolean;
  liveOrderEnabled: boolean;
  accounts: Array<{
    holdings: { items: MeetingHoldingItem[] };
    buyingPower?: Array<{ currency: string; cashBuyingPower: string }>;
    buyingPowerErrors?: string[];
  }>;
};

export type MeetingPositionEvidence = MeetingHoldingItem & {
  evidenceId: string;
  observedAtMs: number;
  provider: string;
  readOnly: boolean;
};

export type MeetingBuyingPowerEvidence = {
  evidenceId: string;
  currency: string;
  cashBuyingPower: string;
  observedAtMs: number;
  provider: string;
  readOnly: boolean;
};

export type HoldingsAnalysisResolution = {
  status: "not_requested" | "resolved" | "portfolio" | "empty" | "too_many";
  query?: string;
  queries?: string[];
  holdings: MeetingHoldingItem[];
  message?: string;
};

export const MAX_PORTFOLIO_ANALYSIS_HOLDINGS = 20;

const normalizeSymbol = (value: string) => value.trim().toUpperCase();

export function analysisRecordTitle(value: string, maxLength = 240) {
  const normalized = value.replace(/\s+/g, " ").trim();
  const characters = Array.from(normalized);
  if (characters.length <= maxLength) return normalized;
  return `${characters.slice(0, Math.max(1, maxLength - 1)).join("")}…`;
}

export function isHoldingsAnalysisRequest(request: string) {
  const normalized = request.replace(/\s+/g, "");
  return /내(?:가)?보유|보유(?:하고있는|중인|한)?종목|내포지션|계좌.*보유/.test(normalized);
}

export function resolveHoldingsAnalysisRequest(
  request: string,
  accountSnapshot: MeetingAccountSnapshot,
): HoldingsAnalysisResolution {
  if (!isHoldingsAnalysisRequest(request)) {
    return { status: "not_requested", holdings: [] };
  }
  const holdings = Array.from(new Map(accountSnapshot.accounts.flatMap((account) => account.holdings.items)
    .map((holding) => [`${holding.currency}:${normalizeSymbol(holding.symbol)}`, holding])).values());
  if (holdings.length === 0) {
    return {
      status: "empty",
      holdings,
      message: "연결된 토스증권 계좌에서 분석할 보유종목이 확인되지 않았습니다.",
    };
  }
  if (holdings.length > MAX_PORTFOLIO_ANALYSIS_HOLDINGS) {
    return {
      status: "too_many",
      holdings,
      message: `보유종목이 ${holdings.length}개로 한 회의의 안전 상한 ${MAX_PORTFOLIO_ANALYSIS_HOLDINGS}개를 넘었습니다. 계좌·시장·종목 범위를 나눠 요청해 주세요. 일부 종목만 임의 분석하지 않았습니다.`,
    };
  }
  if (holdings.length > 1) return {
    status: "portfolio",
    holdings,
    queries: holdings.map((holding) => `${holding.name} (${holding.symbol})`),
    message: `보유종목 ${holdings.length}개를 각각 분석하는 전체 포트폴리오 근거 묶음입니다.`,
  };
  const holding = holdings[0];
  return {
    status: "resolved",
    holdings,
    query: `${request}\n분석 대상 보유종목: ${holding.name} (${holding.symbol})`,
  };
}

export function analysisEvidenceId(snapshotId: string, suffix: string) {
  const safeSnapshotId = snapshotId.toLowerCase().replace(/[^a-z0-9-]+/g, "-").replace(/^-+|-+$/g, "");
  const safeSuffix = suffix.toLowerCase().replace(/[^a-z0-9-]+/g, "-").replace(/^-+|-+$/g, "");
  return `${safeSnapshotId || "snapshot"}-${safeSuffix || "evidence"}`;
}

export function positionEvidenceForSymbol(
  snapshotId: string,
  symbol: string,
  accountSnapshot: MeetingAccountSnapshot,
): MeetingPositionEvidence[] {
  const normalizedSymbol = normalizeSymbol(symbol);
  let sequence = 0;
  return accountSnapshot.accounts.flatMap((account) => account.holdings.items.flatMap((holding) => {
    if (normalizeSymbol(holding.symbol) !== normalizedSymbol) return [];
    sequence += 1;
    return [{
      ...holding,
      evidenceId: analysisEvidenceId(snapshotId, `position-${sequence}`),
      observedAtMs: accountSnapshot.fetchedAtMs,
      provider: accountSnapshot.provider,
      readOnly: accountSnapshot.readOnly,
    }];
  }));
}

export function buyingPowerEvidence(accountSnapshot: MeetingAccountSnapshot): MeetingBuyingPowerEvidence[] {
  let sequence = 0;
  return accountSnapshot.accounts.flatMap((account) => (account.buyingPower ?? []).map((item) => {
    sequence += 1;
    return {
      evidenceId: analysisEvidenceId(`toss-account-${accountSnapshot.fetchedAtMs}`, `buying-power-${sequence}`),
      currency: item.currency,
      cashBuyingPower: item.cashBuyingPower,
      observedAtMs: accountSnapshot.fetchedAtMs,
      provider: accountSnapshot.provider,
      readOnly: accountSnapshot.readOnly,
    };
  }));
}

export function portfolioPositionEvidence(accountSnapshot: MeetingAccountSnapshot): MeetingPositionEvidence[] {
  let sequence = 0;
  return accountSnapshot.accounts.flatMap((account) => account.holdings.items.map((holding) => {
    sequence += 1;
    return {
      ...holding,
      evidenceId: analysisEvidenceId(`toss-account-${accountSnapshot.fetchedAtMs}`, `position-${sequence}`),
      observedAtMs: accountSnapshot.fetchedAtMs,
      provider: accountSnapshot.provider,
      readOnly: accountSnapshot.readOnly,
    };
  }));
}

export function telegramEvidenceId(messageId: number, postedAtMs: number, index: number) {
  return `telegram-${messageId}-${postedAtMs}-${index + 1}`;
}

export type ReportEvidenceFinding = { evidenceIds: string[] };

export function invalidReportEvidenceIds(
  findings: ReportEvidenceFinding[],
  allowedEvidenceIds: Iterable<string>,
) {
  const allowed = new Set(allowedEvidenceIds);
  return Array.from(new Set(
    findings.flatMap((finding) => finding.evidenceIds)
      .filter((evidenceId) => !allowed.has(evidenceId)),
  )).sort();
}

export const SHADOW_RUNTIME_EVIDENCE = {
  evidenceId: "runtime-shadow-only-v1",
  mode: "SHADOW_ONLY",
  liveOrderAllowed: false,
  internalPaperCandidateAllowed: true,
} as const;
