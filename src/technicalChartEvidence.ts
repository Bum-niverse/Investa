export type TechnicalChartAssetClass = "equity" | "crypto_spot" | "securities_future" | "crypto_perpetual";

export type TechnicalChartBar = {
  periodStartMs: number; periodEndMs: number; availableAtMs?: number; ingestedAtMs?: number;
  openMinor: number; highMinor: number; lowMinor: number; closeMinor: number; volume: number; completed: boolean;
  sessionId?: string; contractCode?: string; settlementPriceMinor?: number; markPriceMinor?: number; indexPriceMinor?: number;
  fundingRateBps?: number; fundingTimeMs?: number;
};

export type TechnicalChartAnnotation = {
  id: string; kind: "trend_line" | "horizontal_line" | "vertical_line" | "rectangle"; label: string; color: string;
  startTime: number; endTime: number; startPriceMinor: number; endPriceMinor: number;
  basis: "trade" | "settlement" | "mark" | "index" | "funding" | "contract";
};

export type TechnicalChartEvidence = {
  schemaVersion: "2.0"; sourceSnapshotId: string; provider: string; symbol: string; name: string; market: string;
  assetClass: TechnicalChartAssetClass; currency: string; interval: string; adjusted: boolean; asOfMs: number;
  bars: TechnicalChartBar[]; annotations: TechnicalChartAnnotation[]; method: string; warnings: string[];
};

type TechnicalSnapshot = Omit<TechnicalChartEvidence, "schemaVersion" | "sourceSnapshotId" | "assetClass" | "bars" | "annotations" | "method" | "warnings"> & {
  snapshotId: string; assetClass?: TechnicalChartAssetClass; bars: TechnicalChartBar[];
};
export type PointInTimeValidation = { bars: TechnicalChartBar[]; errors: string[]; warnings: string[] };

const finiteBar = (bar: TechnicalChartBar) => bar.completed && Number.isFinite(bar.periodStartMs) && Number.isFinite(bar.periodEndMs)
  && Number.isFinite(bar.openMinor) && Number.isFinite(bar.closeMinor) && Number.isFinite(bar.lowMinor) && Number.isFinite(bar.highMinor)
  && bar.periodStartMs > 0 && bar.periodEndMs > bar.periodStartMs && bar.lowMinor > 0 && bar.highMinor >= bar.lowMinor
  && bar.openMinor >= bar.lowMinor && bar.openMinor <= bar.highMinor && bar.closeMinor >= bar.lowMinor && bar.closeMinor <= bar.highMinor;
const lowest = (bars: TechnicalChartBar[]) => bars.reduce((selected, bar) => bar.lowMinor < selected.lowMinor ? bar : selected);
const highest = (bars: TechnicalChartBar[]) => bars.reduce((selected, bar) => bar.highMinor > selected.highMinor ? bar : selected);
const horizontal = (id: string, label: string, color: string, first: TechnicalChartBar, last: TechnicalChartBar, price: number, basis: TechnicalChartAnnotation["basis"]): TechnicalChartAnnotation => ({
  id, kind: "horizontal_line", label, color, startTime: first.periodStartMs, endTime: last.periodStartMs,
  startPriceMinor: price, endPriceMinor: price, basis,
});

export const inferTechnicalChartAssetClass = (market: string): TechnicalChartAssetClass => {
  const normalized = market.toLowerCase();
  if (normalized.includes("crypto_futures") || normalized.includes("perpetual")) return "crypto_perpetual";
  if (normalized.includes("securities_futures") || normalized.includes("future")) return "securities_future";
  if (normalized.includes("coin") || normalized.includes("crypto")) return "crypto_spot";
  return "equity";
};

export function validatePointInTimeChartBars(input: TechnicalChartBar[], asOfMs: number, assetClass: TechnicalChartAssetClass): PointInTimeValidation {
  const errors: string[] = []; const warnings: string[] = [];
  const bars = input.filter(finiteBar);
  if (bars.some((bar, index) => index > 0 && bar.periodStartMs <= bars[index - 1].periodStartMs)) errors.push("가격 봉은 중복 없이 시각 오름차순이어야 합니다.");
  if (bars.some((bar) => bar.periodEndMs > asOfMs || (bar.availableAtMs != null && bar.availableAtMs > asOfMs) || (bar.ingestedAtMs != null && bar.ingestedAtMs > asOfMs))) errors.push("분석 기준 시각 뒤에 끝나거나 공개·수집된 봉이 포함되어 있습니다.");
  if (bars.some((bar, index) => index > 0 && bar.periodStartMs < bars[index - 1].periodEndMs)) errors.push("가격 봉 시간 구간이 겹칩니다.");
  if (new Set(bars.map((bar) => bar.periodStartMs)).size !== bars.length) errors.push("동일 시작 시각의 중복 봉이 있습니다.");
  if (assetClass !== "equity" && bars.some((bar) => bar.availableAtMs == null || bar.ingestedAtMs == null)) warnings.push("일부 봉의 공개·수집 시각이 없어 엄격한 시점 재현을 보장할 수 없습니다.");
  if (assetClass === "securities_future" && bars.some((bar) => !bar.contractCode)) errors.push("증권선물 봉에는 만기별 계약코드가 필요합니다.");
  if (assetClass === "securities_future" && bars.length > 0) {
    const currentContract = bars[bars.length - 1].contractCode;
    if (bars.filter((bar) => bar.contractCode === currentContract).length < 20) errors.push("현재 만기 계약의 완료 봉이 20개 미만입니다.");
  }
  if (assetClass === "crypto_perpetual" && bars.some((bar) => bar.markPriceMinor == null || bar.indexPriceMinor == null)) errors.push("코인 무기한선물 봉에는 마크가격과 지수가격이 필요합니다.");
  return { bars: errors.length ? [] : bars.slice(-120), errors, warnings };
}

const observationAnnotations = (bars: TechnicalChartBar[], prefix = "관측"): TechnicalChartAnnotation[] => {
  const observation = bars.slice(-Math.min(60, bars.length)); const support = lowest(observation); const resistance = highest(observation);
  const first = observation[0]; const last = observation[observation.length - 1];
  const annotations: TechnicalChartAnnotation[] = [
    horizontal("observed-low", `${observation.length}봉 ${prefix} 저점`, "#58c99b", first, last, support.lowMinor, "trade"),
    horizontal("observed-high", `${observation.length}봉 ${prefix} 고점`, "#ef7d72", first, last, resistance.highMinor, "trade"),
  ];
  if (observation.length < 2) return annotations;
  const middle = Math.floor(observation.length / 2); const firstLow = lowest(observation.slice(0, middle)); const secondLow = lowest(observation.slice(middle));
  if (firstLow.periodStartMs < secondLow.periodStartMs) annotations.push({ id: "swing-low-trend", kind: "trend_line", label: `${prefix} 저점 연결선`, color: "#d7ee65", startTime: firstLow.periodStartMs, endTime: secondLow.periodStartMs, startPriceMinor: firstLow.lowMinor, endPriceMinor: secondLow.lowMinor, basis: "trade" });
  return annotations;
};
const rangeBox = (bars: TechnicalChartBar[], count: number, label: string): TechnicalChartAnnotation => {
  const recent = bars.slice(-Math.min(count, bars.length));
  return { id: "recent-range", kind: "rectangle", label, color: "#79aee8", startTime: recent[0].periodStartMs, endTime: recent[recent.length - 1].periodStartMs, startPriceMinor: lowest(recent).lowMinor, endPriceMinor: highest(recent).highMinor, basis: "trade" };
};

const securitiesFutureAnnotations = (bars: TechnicalChartBar[]): TechnicalChartAnnotation[] => {
  let contractStart = 0;
  for (let index = 1; index < bars.length; index += 1) if (bars[index].contractCode !== bars[index - 1].contractCode) contractStart = index;
  const current = bars.slice(contractStart); const last = current[current.length - 1];
  const annotations = [...observationAnnotations(current, `${last.contractCode} 계약`)];
  if (last.settlementPriceMinor != null) annotations.push(horizontal("latest-settlement", "최근 공식 정산가", "#f2b45d", current[0], last, last.settlementPriceMinor, "settlement"));
  bars.forEach((bar, index) => { if (index > 0 && bar.contractCode !== bars[index - 1].contractCode) annotations.push({ id: `roll-${bar.periodStartMs}`, kind: "vertical_line", label: `${bars[index - 1].contractCode} → ${bar.contractCode} 롤 경계`, color: "#b982e6", startTime: bar.periodStartMs, endTime: bar.periodStartMs, startPriceMinor: lowest(bars).lowMinor, endPriceMinor: highest(bars).highMinor, basis: "contract" }); });
  annotations.push(rangeBox(current, 20, `현 계약 최근 ${Math.min(20, current.length)}봉 범위`));
  return annotations;
};

const cryptoPerpetualAnnotations = (bars: TechnicalChartBar[]): TechnicalChartAnnotation[] => {
  const annotations = [...observationAnnotations(bars, "24시간")]; const first = bars[0]; const last = bars[bars.length - 1];
  annotations.push(horizontal("latest-mark", "마크가격", "#e4c65a", first, last, last.markPriceMinor!, "mark"));
  annotations.push(horizontal("latest-index", "지수가격", "#63c7d6", first, last, last.indexPriceMinor!, "index"));
  bars.filter((bar) => bar.fundingTimeMs != null).slice(-8).forEach((bar) => annotations.push({ id: `funding-${bar.fundingTimeMs}`, kind: "vertical_line", label: `펀딩 ${bar.fundingRateBps == null ? "관측" : `${bar.fundingRateBps}bp`}`, color: "#d68ad9", startTime: bar.fundingTimeMs!, endTime: bar.fundingTimeMs!, startPriceMinor: lowest(bars).lowMinor, endPriceMinor: highest(bars).highMinor, basis: "funding" }));
  annotations.push(rangeBox(bars, 30, `24시간 연속시장 최근 ${Math.min(30, bars.length)}봉 범위`));
  return annotations;
};

export function buildTechnicalChartAnnotations(input: TechnicalChartBar[], assetClass: TechnicalChartAssetClass = "equity"): TechnicalChartAnnotation[] {
  const bars = input.filter(finiteBar).slice(-120); if (bars.length < 2) return [];
  if (assetClass === "securities_future") return securitiesFutureAnnotations(bars);
  if (assetClass === "crypto_perpetual") return cryptoPerpetualAnnotations(bars);
  if (assetClass === "crypto_spot") return [...observationAnnotations(bars, "24시간"), rangeBox(bars, 30, `24시간 연속시장 최근 ${Math.min(30, bars.length)}봉 범위`)];
  return [...observationAnnotations(bars), rangeBox(bars, 20, `최근 ${Math.min(20, bars.length)}봉 가격 범위`)];
}

const evidenceMethod = (assetClass: TechnicalChartAssetClass) => ({
  equity: "완료 봉의 관측 고·저점, 저점 연결선과 최근 20봉 가격 범위를 결정론적으로 표시",
  crypto_spot: "24시간 연속시장의 완료 봉만 사용해 관측 고·저점, 저점 연결선과 최근 30봉 범위를 표시하며 거래가 없던 누락 봉을 임의 생성하지 않음",
  securities_future: "현재 만기 계약 안에서만 고·저점과 추세를 계산하고 공식 정산가와 계약 롤 경계를 별도 표시",
  crypto_perpetual: "24시간 완료 봉의 가격 구조와 마크가격·지수가격·펀딩 시점을 서로 다른 근거선으로 표시",
}[assetClass]);

export function buildTechnicalChartEvidence(snapshot: TechnicalSnapshot): TechnicalChartEvidence | null {
  const assetClass = snapshot.assetClass ?? inferTechnicalChartAssetClass(snapshot.market);
  const validation = validatePointInTimeChartBars(snapshot.bars, snapshot.asOfMs, assetClass);
  if (validation.errors.length || validation.bars.length < 20) return null;
  return { schemaVersion: "2.0", sourceSnapshotId: snapshot.snapshotId, provider: snapshot.provider, symbol: snapshot.symbol, name: snapshot.name, market: snapshot.market, assetClass, currency: snapshot.currency, interval: snapshot.interval, adjusted: snapshot.adjusted, asOfMs: snapshot.asOfMs, bars: validation.bars, annotations: buildTechnicalChartAnnotations(validation.bars, assetClass), method: evidenceMethod(assetClass), warnings: [
    ...validation.warnings, "표시선은 관측 구간을 설명하는 시각 보조 자료이며 미래 가격 예측이나 주문 신호가 아닙니다.", "수동으로 그린 개인 차트 선과 분리된 불변 분석 기록입니다.",
    ...(assetClass === "securities_future" ? ["연속선물 보정값으로 만기 간 가격 차이를 숨기지 않으며 계약 경계를 넘어 추세선을 연결하지 않습니다."] : []),
    ...(assetClass === "crypto_perpetual" ? ["청산·증거금 판단에는 체결 종가가 아니라 거래소가 제공한 마크가격 계약을 별도로 사용해야 합니다."] : []),
  ] };
}
