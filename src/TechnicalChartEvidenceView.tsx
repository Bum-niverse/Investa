import type { TechnicalChartEvidence } from "./technicalChartEvidence";

const formatMoney = (minor: number, currency: string) => new Intl.NumberFormat("ko-KR", {
  style: "currency",
  currency,
  maximumFractionDigits: currency === "KRW" ? 0 : 2,
}).format(minor / (currency === "KRW" ? 1 : 100));

export function TechnicalChartEvidenceView({ evidence }: { evidence: TechnicalChartEvidence }) {
  const width = 960;
  const height = 440;
  const plot = { left: 18, right: 82, top: 24, bottom: 42 };
  const bars = evidence.bars;
  const minimum = Math.min(...bars.map((bar) => bar.lowMinor));
  const maximum = Math.max(...bars.map((bar) => bar.highMinor));
  const range = Math.max(1, maximum - minimum);
  const plotWidth = width - plot.left - plot.right;
  const plotHeight = height - plot.top - plot.bottom;
  const step = plotWidth / Math.max(1, bars.length);
  const xForIndex = (index: number) => plot.left + step * index + step / 2;
  const indexForTime = (time: number) => {
    const index = bars.findIndex((bar) => bar.periodStartMs >= time);
    return index < 0 ? bars.length - 1 : index;
  };
  const x = (time: number) => xForIndex(indexForTime(time));
  const y = (price: number) => plot.top + (maximum - price) / range * plotHeight;

  return <figure className="technical-chart-evidence">
    <figcaption>
      <div><span>IMMUTABLE CHART CAPTURE · {evidence.provider}</span><h3>{evidence.name} · {evidence.symbol}</h3></div>
      <strong>{new Date(evidence.asOfMs).toLocaleString("ko-KR")} 기준</strong>
    </figcaption>
    <svg viewBox={`0 0 ${width} ${height}`} role="img" aria-label={`${evidence.name} ${evidence.interval} 차트에 기술적 분석가의 관측선 ${evidence.annotations.length}개가 표시된 분석 캡처`}>
      <rect width={width} height={height} fill="#09100c" />
      {[0, 1, 2, 3, 4].map((line) => {
        const price = maximum - range * line / 4;
        const lineY = plot.top + plotHeight * line / 4;
        return <g key={line}><line x1={plot.left} x2={width - plot.right} y1={lineY} y2={lineY} stroke="#26342c" /><text x={width - 8} y={lineY + 4} textAnchor="end">{formatMoney(Math.round(price), evidence.currency)}</text></g>;
      })}
      {bars.map((bar, index) => {
        const up = bar.closeMinor >= bar.openMinor;
        const color = up ? "#e56c68" : "#6697df";
        const bodyTop = y(Math.max(bar.openMinor, bar.closeMinor));
        return <g key={bar.periodStartMs}><line x1={xForIndex(index)} x2={xForIndex(index)} y1={y(bar.highMinor)} y2={y(bar.lowMinor)} stroke={color} /><rect x={xForIndex(index) - Math.max(1.5, step * .28)} y={bodyTop} width={Math.max(3, step * .56)} height={Math.max(2, Math.abs(y(bar.openMinor) - y(bar.closeMinor)))} fill={color} /></g>;
      })}
      {evidence.annotations.map((annotation) => annotation.kind === "rectangle"
        ? <g key={annotation.id}><rect x={Math.min(x(annotation.startTime), x(annotation.endTime))} y={y(annotation.endPriceMinor)} width={Math.max(2, Math.abs(x(annotation.endTime) - x(annotation.startTime)))} height={Math.max(2, Math.abs(y(annotation.startPriceMinor) - y(annotation.endPriceMinor)))} fill={`${annotation.color}18`} stroke={annotation.color} strokeDasharray="6 4" /><text x={Math.min(x(annotation.startTime), x(annotation.endTime)) + 6} y={y(annotation.endPriceMinor) + 16} fill={annotation.color}>{annotation.label}</text></g>
        : <g key={annotation.id}><line x1={x(annotation.startTime)} x2={x(annotation.endTime)} y1={y(annotation.startPriceMinor)} y2={y(annotation.endPriceMinor)} stroke={annotation.color} strokeWidth="2.2" strokeDasharray={annotation.kind === "horizontal_line" ? "7 4" : undefined} vectorEffect="non-scaling-stroke" /><rect x={Math.max(plot.left, x(annotation.endTime) - 132)} y={Math.max(plot.top, y(annotation.endPriceMinor) - 20)} width="128" height="17" fill="#09100cdd" /><text x={x(annotation.endTime) - 8} y={Math.max(plot.top + 12, y(annotation.endPriceMinor) - 8)} fill={annotation.color} textAnchor="end">{annotation.label}</text></g>)}
      <text x={plot.left} y={height - 14}>{evidence.adjusted ? "수정주가" : "원주가"} · {bars.length}개 완료 봉 · snapshot {evidence.sourceSnapshotId}</text>
    </svg>
    <div className="technical-chart-method"><strong>선 산출 기준</strong><p>{evidence.method}</p><ul>{evidence.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul></div>
  </figure>;
}
