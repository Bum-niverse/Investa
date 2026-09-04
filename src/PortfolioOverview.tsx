import type { MeetingBuyingPowerEvidence, MeetingPositionEvidence } from "./meetingEvidence";
import { buildPortfolioCurrencyGroups, formatPortfolioMoney } from "./portfolioPresentation";

export type PortfolioRecordSnapshot = {
  provider: string;
  observedAtMs: number;
  readOnly: true;
  positions: MeetingPositionEvidence[];
  buyingPower: MeetingBuyingPowerEvidence[];
  warnings: string[];
};

export function PortfolioOverview({ snapshot, title = "보유자산 구성", emptyMessage = "조회된 보유자산이 없습니다." }: {
  snapshot: PortfolioRecordSnapshot;
  title?: string;
  emptyMessage?: string;
}) {
  const groups = buildPortfolioCurrencyGroups(snapshot.positions, snapshot.buyingPower);
  return <section className="portfolio-overview" aria-labelledby={`portfolio-${snapshot.observedAtMs}`}>
    <header>
      <div><span>READ ONLY · {snapshot.provider}</span><h3 id={`portfolio-${snapshot.observedAtMs}`}>{title}</h3></div>
      <small>{snapshot.observedAtMs > 0 ? `${new Date(snapshot.observedAtMs).toLocaleString("ko-KR")} 기준` : "관측 시각 확인 실패"}</small>
    </header>
    {groups.length === 0 ? <p className="portfolio-empty">{emptyMessage}</p> : <div className="portfolio-currency-groups">
      {groups.map((group) => <article className="portfolio-currency-card" key={group.currency}>
        <div className="portfolio-chart-layout">
          <div className="portfolio-donut" style={{ background: group.gradient }} role="img" aria-label={`${group.currency} 보유 평가액 구성`}>
            <div><small>보유 평가액</small><strong>{formatPortfolioMoney(group.marketValue, group.currency)}</strong></div>
          </div>
          <div className="portfolio-summary">
            <div><span>{group.currency} 평가손익</span><strong className={group.profitLoss >= 0 ? "is-positive" : "is-negative"}>{formatPortfolioMoney(group.profitLoss, group.currency)}</strong></div>
            <div><span>현금 기반 매수 가능</span><strong>{group.buyingPower == null ? "미제공" : formatPortfolioMoney(group.buyingPower, group.currency)}</strong></div>
            <div className="portfolio-legend" role="list">
              {group.slices.map((slice) => <div key={slice.id} role="listitem"><i style={{ backgroundColor: slice.color }} aria-hidden="true" /><b>{slice.symbol}</b><span>{slice.name}</span><strong>{(slice.weightBps / 100).toFixed(1)}%</strong></div>)}
            </div>
          </div>
        </div>
        <div className="portfolio-allocation-bar" aria-hidden="true">{group.slices.map((slice) => <i key={slice.id} style={{ width: `${slice.weightBps / 100}%`, backgroundColor: slice.color }} />)}</div>
        <div className="portfolio-table-wrap"><table><thead><tr><th>종목</th><th>수량</th><th>평가액</th><th>평가손익</th><th>비중</th></tr></thead><tbody>{group.slices.map((slice) => <tr key={slice.id}><td><strong>{slice.symbol}</strong><small>{slice.name}</small></td><td>{slice.isOther ? "-" : slice.quantity.toLocaleString("ko-KR", { maximumFractionDigits: 8 })}</td><td>{formatPortfolioMoney(slice.marketValue, group.currency)}</td><td className={slice.profitLoss >= 0 ? "is-positive" : "is-negative"}>{formatPortfolioMoney(slice.profitLoss, group.currency)}</td><td>{(slice.weightBps / 100).toFixed(1)}%</td></tr>)}</tbody></table></div>
      </article>)}
    </div>}
    {snapshot.warnings.length > 0 && <ul className="portfolio-warnings">{snapshot.warnings.map((warning, index) => <li key={`${index}-${warning}`}>{warning}</li>)}</ul>}
    <p className="portfolio-footnote">업종·예상 배당은 현재 계좌 공급 데이터에 없어 추정하지 않습니다. 서로 다른 통화도 환율 근거 없이 합산하지 않습니다.</p>
  </section>;
}
