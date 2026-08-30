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
  }>;
};

export type MeetingPositionEvidence = MeetingHoldingItem & {
  evidenceId: string;
  observedAtMs: number;
  provider: string;
  readOnly: boolean;
};

const normalizeSymbol = (value: string) => value.trim().toUpperCase();

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
