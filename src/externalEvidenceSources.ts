export type ExternalEvidenceSource = {
  evidenceId: string;
  provider: "NAVER_NEWS_SEARCH" | "TELEGRAM_USER_SELECTED_CHANNELS" | "OPENDART";
  medium: "news" | "telegram" | "disclosure";
  sourceName: string;
  title: string;
  sourceUrl?: string | null;
  platformUrl?: string | null;
  publishedAt?: string | null;
  observedAtMs: number;
  cited: boolean;
};

const safeHost = (value: string) => {
  try {
    const parsed = new URL(value);
    if (parsed.protocol !== "https:" && parsed.protocol !== "http:") return "언론사 확인 불가";
    return parsed.hostname.replace(/^www\./, "") || "언론사 확인 불가";
  } catch {
    return "언론사 확인 불가";
  }
};

export const naverNewsEvidenceSource = (
  evidenceId: string,
  item: { title: string; originalLink: string; link: string; publishedAt: string },
  fetchedAtMs: number,
): ExternalEvidenceSource => ({
  evidenceId,
  provider: "NAVER_NEWS_SEARCH",
  medium: "news",
  sourceName: safeHost(item.originalLink || item.link),
  title: item.title,
  sourceUrl: item.originalLink || item.link || null,
  platformUrl: item.link && item.link !== item.originalLink ? item.link : null,
  publishedAt: item.publishedAt || null,
  observedAtMs: fetchedAtMs,
  cited: false,
});

export const telegramEvidenceSource = (
  evidenceId: string,
  item: { sourceTitle: string; sourceUsername?: string | null; messageId: number; postedAtMs: number; text: string },
  observedAtMs: number,
): ExternalEvidenceSource => {
  const username = item.sourceUsername?.replace(/^@/, "") ?? "";
  const publicUrl = /^[A-Za-z0-9_]{5,32}$/.test(username)
    ? `https://t.me/${username}/${item.messageId}`
    : null;
  const title = Array.from(item.text.replace(/\s+/g, " ").trim()).slice(0, 140).join("");
  return {
    evidenceId,
    provider: "TELEGRAM_USER_SELECTED_CHANNELS",
    medium: "telegram",
    sourceName: item.sourceUsername ? `${item.sourceTitle} · @${username}` : item.sourceTitle,
    title: title || `Telegram 메시지 #${item.messageId}`,
    sourceUrl: publicUrl,
    platformUrl: null,
    publishedAt: new Date(item.postedAtMs).toISOString(),
    observedAtMs,
    cited: false,
  };
};

export const disclosureEvidenceSource = (
  evidenceId: string,
  item: { corporationName: string; reportName: string; receiptDate: string; sourceUrl: string },
  observedAtMs: number,
): ExternalEvidenceSource => ({
  evidenceId,
  provider: "OPENDART",
  medium: "disclosure",
  sourceName: `전자공시시스템 DART · ${item.corporationName}`,
  title: item.reportName,
  sourceUrl: item.sourceUrl || null,
  platformUrl: null,
  publishedAt: item.receiptDate || null,
  observedAtMs,
  cited: false,
});

export const markCitedEvidenceSources = (
  sources: ExternalEvidenceSource[],
  citedEvidenceIds: Iterable<string>,
) => {
  const cited = new Set(citedEvidenceIds);
  const byId = new Map<string, ExternalEvidenceSource>();
  for (const source of sources) {
    const next = { ...source, cited: source.cited || cited.has(source.evidenceId) };
    const existing = byId.get(source.evidenceId);
    if (!existing || (!existing.cited && next.cited)) byId.set(source.evidenceId, next);
  }
  return Array.from(byId.values()).sort((left, right) => right.observedAtMs - left.observedAtMs || left.evidenceId.localeCompare(right.evidenceId));
};
