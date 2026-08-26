export type TechnicalChartBar = {
  periodStartMs: number;
  periodEndMs: number;
  openMinor: number;
  highMinor: number;
  lowMinor: number;
  closeMinor: number;
  volume: number;
  completed: boolean;
};

export type TechnicalChartAnnotation = {
  id: string;
  kind: "trend_line" | "horizontal_line" | "rectangle";
  label: string;
  color: string;
  startTime: number;
  endTime: number;
  startPriceMinor: number;
  endPriceMinor: number;
};

export type TechnicalChartEvidence = {
  schemaVersion: "1.0";
  sourceSnapshotId: string;
  provider: string;
  symbol: string;
  name: string;
  market: string;
  currency: string;
  interval: "1d";
  adjusted: boolean;
  asOfMs: number;
  bars: TechnicalChartBar[];
  annotations: TechnicalChartAnnotation[];
  method: string;
  warnings: string[];
};

type TechnicalSnapshot = Omit<TechnicalChartEvidence, "schemaVersion" | "sourceSnapshotId" | "bars" | "annotations" | "method" | "warnings"> & {
  snapshotId: string;
  bars: TechnicalChartBar[];
};

const finiteBar = (bar: TechnicalChartBar) => bar.completed
  && Number.isFinite(bar.periodStartMs)
  && Number.isFinite(bar.lowMinor)
  && Number.isFinite(bar.highMinor)
  && bar.periodStartMs > 0
  && bar.lowMinor > 0
  && bar.highMinor >= bar.lowMinor;

const lowest = (bars: TechnicalChartBar[]) => bars.reduce((selected, bar) => bar.lowMinor < selected.lowMinor ? bar : selected);
const highest = (bars: TechnicalChartBar[]) => bars.reduce((selected, bar) => bar.highMinor > selected.highMinor ? bar : selected);

export function buildTechnicalChartAnnotations(input: TechnicalChartBar[]): TechnicalChartAnnotation[] {
  const bars = input.filter(finiteBar).slice(-120);
  if (bars.length < 2) return [];

  const annotations: TechnicalChartAnnotation[] = [];
  const observation = bars.slice(-Math.min(60, bars.length));
  const support = lowest(observation);
  const resistance = highest(observation);
  const firstTime = observation[0].periodStartMs;
  const lastTime = observation[observation.length - 1].periodStartMs;
  annotations.push({
    id: "observed-low",
    kind: "horizontal_line",
    label: `${observation.length}봉 관측 저점`,
    color: "#58c99b",
    startTime: firstTime,
    endTime: lastTime,
    startPriceMinor: support.lowMinor,
    endPriceMinor: support.lowMinor,
  });
  annotations.push({
    id: "observed-high",
    kind: "horizontal_line",
    label: `${observation.length}봉 관측 고점`,
    color: "#ef7d72",
    startTime: firstTime,
    endTime: lastTime,
    startPriceMinor: resistance.highMinor,
    endPriceMinor: resistance.highMinor,
  });

  const trendBars = bars.slice(-Math.min(80, bars.length));
  const middle = Math.floor(trendBars.length / 2);
  const firstLow = lowest(trendBars.slice(0, middle));
  const secondLow = lowest(trendBars.slice(middle));
  if (firstLow.periodStartMs < secondLow.periodStartMs) {
    annotations.push({
      id: "swing-low-trend",
      kind: "trend_line",
      label: "구간 저점 연결선",
      color: "#d7ee65",
      startTime: firstLow.periodStartMs,
      endTime: secondLow.periodStartMs,
      startPriceMinor: firstLow.lowMinor,
      endPriceMinor: secondLow.lowMinor,
    });
  }

  const recent = bars.slice(-Math.min(20, bars.length));
  annotations.push({
    id: "recent-range",
    kind: "rectangle",
    label: `최근 ${recent.length}봉 가격 범위`,
    color: "#79aee8",
    startTime: recent[0].periodStartMs,
    endTime: recent[recent.length - 1].periodStartMs,
    startPriceMinor: lowest(recent).lowMinor,
    endPriceMinor: highest(recent).highMinor,
  });
  return annotations;
}

export function buildTechnicalChartEvidence(snapshot: TechnicalSnapshot): TechnicalChartEvidence | null {
  const bars = snapshot.bars.filter(finiteBar).slice(-120);
  if (bars.length < 20) return null;
  return {
    schemaVersion: "1.0",
    sourceSnapshotId: snapshot.snapshotId,
    provider: snapshot.provider,
    symbol: snapshot.symbol,
    name: snapshot.name,
    market: snapshot.market,
    currency: snapshot.currency,
    interval: snapshot.interval,
    adjusted: snapshot.adjusted,
    asOfMs: snapshot.asOfMs,
    bars,
    annotations: buildTechnicalChartAnnotations(bars),
    method: "완료 봉의 최근 60봉 고·저점, 두 시간 구간의 저점 연결선과 최근 20봉 가격 범위를 결정론적으로 표시",
    warnings: [
      "표시선은 관측 구간을 설명하는 시각 보조 자료이며 미래 가격 예측이나 주문 신호가 아닙니다.",
      "수동으로 그린 개인 차트 선과 분리된 불변 분석 기록입니다.",
    ],
  };
}
