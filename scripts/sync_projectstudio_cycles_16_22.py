"""Investa 16~22차 검증 결과를 ProjectStudio 로컬 문서에 멱등 반영한다."""

from __future__ import annotations

import argparse
import json
import sqlite3
from datetime import datetime, timezone


PROJECT_ID = "36e87491-74a8-48ca-a7b8-30fa6ccea131"
PRD_DOCUMENT_ID = "58cf42a6-d2ce-46d3-996b-e7ebebaa8890"
REVISION_ID = "revision-investa-contract-cycles-16-22"
ACTIVITY_ID = "activity-investa-contract-cycles-16-22"

MET_CRITERIA = {
    "ac-agents-specialists-1": "src-tauri/src/codex.rs",
    "ac-data-freshness-2": "src-tauri/src/data_quality.rs",
    "ac-performance-metrics-1": "src-tauri/src/performance.rs",
    "ac-performance-attribution-2": "src-tauri/src/performance.rs",
    "ac-crypto-risk-gate-2": "src-tauri/src/crypto_contracts.rs",
    "ac-trading-room-drawer-2": "src/App.tsx",
}

FEATURE_STATUS = {
    "feat-data-freshness": "done",
    "feat-performance-metrics": "done",
    "feat-performance-attribution": "done",
    "feat-agents-specialists": "in_progress",
    "feat-crypto-risk-gate": "in_progress",
    "feat-trading-room-drawer": "in_progress",
    "req-decision": "in_progress",
    "req-research": "in_progress",
    "req-performance": "in_progress",
    "req-crypto": "in_progress",
    "req-project-records": "in_progress",
    "req-screening": "in_progress",
}

PRD_SECTION = """

## 34. 계약 강화 사이클 16~22 (2026-08-25)

- 직원 개별 소견은 중복 없는 근거 ID, 출처, 선택적 원천 리비전·관측 시각과 반대 근거를 기록한다. 부서 종합은 직원별 근거 ID를 유지하고 근거가 없으면 근거 공백을 필수로 남긴다.
- 필수 데이터가 누락·지연돼 대체 출처를 쓰면 필수·대체 출처, 기준 시각, 대체 리비전, 값 차이와 사유를 기록한다. 대체 출처 결과는 분석 전용이며 주문 승격은 차단한다.
- 코인 파생 승격 기반은 OOS 표본, 비용 차감 기대손익, Profit Factor, MDD, 청산 0회, 불리한 펀딩비, 변동성 충격, API 지연 복구와 재시작 대사를 모두 검사한다. 통과해도 섀도우 검토만 허용하고 실주문은 잠근다.
- 성과는 KRW·USD·USDT 등 통화별 실현·미실현·비용·순손익으로 분리한다. 시점 정합 환율이 없으면 기준통화 총계를 만들지 않는다.
- 직원·전략 평가는 단순 적중률보다 Brier 보정 오차, 양의 기대값 건수와 실현 순손익을 우선 기록한다.
- 에이전트 상세 패널은 닫힌 상태에서 키보드 탐색을 차단하고 Escape로 닫은 뒤 선택했던 직원 버튼으로 포커스를 복귀시킨다.
- 검증: TypeScript·Vite 프로덕션 빌드 통과, Rust 171개 중 167개 통과·외부 자격정보 및 네트워크 통합 검사 4개 명시적 제외. 신규 라이브러리, 외부 계정, 유료 공급자와 실주문은 추가하지 않았다.
"""


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("database")
    args = parser.parse_args()
    now = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
    connection = sqlite3.connect(args.database)
    connection.execute("PRAGMA foreign_keys = ON")
    if connection.execute("PRAGMA quick_check").fetchone() != ("ok",):
        raise RuntimeError("ProjectStudio DB quick_check 실패")
    if connection.execute("PRAGMA foreign_key_check").fetchall():
        raise RuntimeError("ProjectStudio DB foreign_key_check 실패")

    with connection:
        for criterion_id, target_ref in MET_CRITERIA.items():
            updated = connection.execute(
                "UPDATE acceptance_criteria SET is_met = 1, updated_at = ? WHERE id = ?",
                (now, criterion_id),
            ).rowcount
            if updated != 1:
                raise RuntimeError(f"수용 기준을 찾지 못했습니다: {criterion_id}")
            connection.execute(
                """INSERT OR IGNORE INTO trace_links
                   (id, project_id, source_type, source_id, target_type, target_ref, metadata_json, created_at)
                   VALUES (?, ?, 'criterion', ?, 'file', ?, ?, ?)""",
                (
                    f"trace-{criterion_id}",
                    PROJECT_ID,
                    criterion_id,
                    target_ref,
                    json.dumps({"cycle": "16-22", "verification": "cargo test + pnpm build"}, ensure_ascii=False),
                    now,
                ),
            )

        for feature_id, status in FEATURE_STATUS.items():
            updated = connection.execute(
                "UPDATE features SET status = ?, updated_at = ? WHERE id = ? AND project_id = ?",
                (status, now, feature_id, PROJECT_ID),
            ).rowcount
            if updated != 1:
                raise RuntimeError(f"기능 노드를 찾지 못했습니다: {feature_id}")

        current = connection.execute(
            """SELECT r.revision_number, r.content_markdown, r.content_json
               FROM documents d JOIN document_revisions r ON r.id = d.current_revision_id
               WHERE d.id = ? AND d.project_id = ?""",
            (PRD_DOCUMENT_ID, PROJECT_ID),
        ).fetchone()
        if current is None:
            raise RuntimeError("Investa PRD 현재 리비전을 찾지 못했습니다.")
        if "## 34. 계약 강화 사이클 16~22" not in current[1]:
            connection.execute(
                """INSERT INTO document_revisions
                   (id, document_id, revision_number, content_markdown, content_json, source, created_at)
                   VALUES (?, ?, ?, ?, ?, 'ai', ?)""",
                (REVISION_ID, PRD_DOCUMENT_ID, current[0] + 1, current[1].rstrip() + PRD_SECTION, current[2], now),
            )
            connection.execute(
                "UPDATE documents SET current_revision_id = ?, updated_at = ? WHERE id = ?",
                (REVISION_ID, now, PRD_DOCUMENT_ID),
            )

        connection.execute(
            """INSERT OR IGNORE INTO activity_log
               (id, project_id, action, target_type, target_id, details_json, created_at)
               VALUES (?, ?, ?, 'document', ?, ?, ?)""",
            (
                ACTIVITY_ID,
                PROJECT_ID,
                "Investa 계약 강화 16~22차 구현·검증 반영",
                PRD_DOCUMENT_ID,
                json.dumps(
                    {
                        "criteria": sorted(MET_CRITERIA),
                        "tests": {"rustPassed": 167, "rustIgnoredExternal": 4, "frontendBuild": "passed"},
                        "liveOrderEnabled": False,
                    },
                    ensure_ascii=False,
                ),
                now,
            ),
        )
        connection.execute("UPDATE projects SET updated_at = ? WHERE id = ?", (now, PROJECT_ID))

    if connection.execute("PRAGMA quick_check").fetchone() != ("ok",):
        raise RuntimeError("ProjectStudio DB 변경 후 quick_check 실패")
    if connection.execute("PRAGMA foreign_key_check").fetchall():
        raise RuntimeError("ProjectStudio DB 변경 후 foreign_key_check 실패")
    connection.close()


if __name__ == "__main__":
    main()
