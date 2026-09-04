"""직원별 Agent v2와 안전 도구 루프 계획을 ProjectStudio에 멱등 반영한다."""

from __future__ import annotations

import argparse
import copy
import importlib.util
import json
from pathlib import Path


PROJECT_ID = "36e87491-74a8-48ca-a7b8-30fa6ccea131"
FEATURE_IDS = {"feat-research-employee-agent-v2", "feat-strategy-risk-employee-agent-v2", "feat-remaining-departments-employee-agent-v2", "feat-agent-tool-plan-broker", "feat-corporate-action-official-evidence-calibration", "feat-user-triggered-codex-web-research", "feat-department-report-detail-view"}
FLOW_NODE_ID = "flow-meeting-analysis-cycle-3"
SECTION_MARKER = "직원별 Agent v2와 안전 도구 브로커"
DETAIL_SECTION_MARKER = "부서별 장문 보고와 공식 근거 보강"


def load_api(root: Path):
    source = root / "scripts" / "projectstudio_api.py"
    spec = importlib.util.spec_from_file_location("projectstudio_api", source)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"ProjectStudio API를 불러오지 못했습니다: {source}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def feature(feature_id: str, title: str, description: str, status: str, sort_order: int, criteria):
    return {
        "id": feature_id,
        "parentId": "feat-agents-trace",
        "title": title,
        "description": description,
        "status": status,
        "priority": "high",
        "role": "AI 오케스트레이션 담당 · 부서 총괄 · 보안 담당",
        "sortOrder": sort_order,
        "colorKey": "green" if status == "done" else "amber",
        "acceptanceCriteria": [
            {"id": f"{feature_id}-ac-{index + 1}", "description": text, "isMet": met, "sortOrder": index}
            for index, (text, met) in enumerate(criteria)
        ],
    }


def desired_features():
    return [
        feature(
            "feat-research-employee-agent-v2",
            "리서치부 직원별 독립 Agent 실행",
            "전체 회의에서 리서치부 다섯 직원이 별도 RoleReport를 만들고 부장이 실제 결과만 취합한다.",
            "done",
            648,
            [
                ("기술·펀더멘털·뉴스·거시·논문 연구원을 서로 다른 Codex turn으로 실행한다.", True),
                ("직원 실패를 빈 성공으로 숨기지 않고 결정론적 근거 공백으로 부장에게 전달한다.", True),
                ("부장 보고의 직원 ID와 근거 ID를 실제 직원 결과 및 회의 근거 허용 목록과 대조한다.", True),
                ("리서치부 11회 비용과 분류·본부장 종합 비용을 호출 예산에 반영한다.", True),
                ("직원 RoleReport·부서 취합·본부장 종합은 5분, 안건 분류·도구 계획은 3분을 넘으면 실제 turn을 취소하고 단계별 실패로 닫는다.", True),
                ("직원 보고에는 실제 도구가 반환한 evidenceId 허용 배열을 주입하고 임시·mv-* 근거 생성을 금지한다.", True),
                ("부서장은 계약 상한으로 축약된 직원 결과만 받아 medium 추론으로 취합한다.", True),
                ("부서 단위 기능 플래그로 점진 전환과 배치 보고 롤백이 가능하다.", True),
            ],
        ),
        feature(
            "feat-remaining-departments-employee-agent-v2",
            "운영·디지털자산·홍보·투자공학·준법 직원별 독립 Agent 실행",
            "남은 5개 부서 직원이 비식별 운영·원장·감사·근거 manifest 도구를 선택해 독립 RoleReport를 만들고 부서장이 검증된 결과만 취합한다.",
            "done",
            650,
            [
                ("매매운영·디지털자산·홍보·투자공학·준법 직원 21명을 독립 Codex turn으로 실행한다.", True),
                ("운영·모의원장·감사·근거 manifest 도구는 읽기 전용이며 계좌 ID와 감사 상세 원문을 제거한다.", True),
                ("온체인·미디어 등 공급되지 않은 자료는 추정하지 않고 근거 공백으로 보고한다.", True),
                ("모든 전문 부서가 같은 AgentToolPlan·RoleReport·DepartmentReport 검증 경로를 사용한다.", True),
                ("중요 복합 회의는 최대 80회 상한 안에서 소집된 전문 부서의 직원을 모두 독립 실행하며 부장 대리 소견을 만들지 않는다.", True),
            ],
        ),
        feature(
            "feat-strategy-risk-employee-agent-v2",
            "전략운용·리스크관리 직원별 독립 Agent 실행",
            "전략운용부와 리스크관리부 직원이 역할별 도구를 선택해 별도 RoleReport를 만들고 부장이 실제 결과만 취합한다.",
            "done",
            649,
            [
                ("Bull·Bear·트레이더·백테스트 연구원을 서로 다른 Codex turn으로 실행한다.", True),
                ("공격·중립·보수 위험, 한도 감시와 독립 모델검증을 서로 다른 Codex turn으로 실행한다.", True),
                ("포지션 도구는 계좌 식별자를 제거한 읽기 전용 종목·현금·운용 원칙만 제공한다.", True),
                ("복합 회의는 소집된 부서의 실제 직원 RoleReport만 취합하고 80회 상한·2명 동시성·80% 사용량 중단선을 적용한다.", True),
                ("직원 보고의 근거 ID와 역할 범위를 Rust·프런트·부장 취합에서 다시 검증한다.", True),
            ],
        ),
        feature(
            "feat-agent-tool-plan-broker",
            "Agent 선택형 읽기 전용 도구 루프",
            "직원이 역할별 허용 도구를 선택하면 Rust가 인자·횟수·출력을 검증해 실행하고 결과를 다시 RoleReport에 전달한다.",
            "done",
            651,
            [
                ("Crossref 공개 서지 메타데이터를 고정 호스트·timeout·건수 제한으로 수집한다.", True),
                ("외부 자료를 신뢰할 수 없는 입력으로 격리하고 원문 검증·성과로 과장하지 않는다.", True),
                ("AgentToolPlan 구조화 계약과 역할별 도구 allowlist를 구현한다.", True),
                ("도구 호출 전후 guardrail, 횟수·크기 한도와 추적 로그를 구현한다.", True),
                ("도구 결과를 받은 두 번째 turn만 최종 RoleReport로 승인한다.", True),
            ],
        ),
        feature(
            "feat-corporate-action-official-evidence-calibration",
            "기업행위 공식 근거와 신뢰도 보정",
            "국내 회사분할 안건은 OpenDART 구조화 공시를 별도 조회하고 핵심 공식 근거가 없으면 근거 충족도를 35% 이하로 제한한다.",
            "done",
            652,
            [
                ("OpenDART 회사분할 결정 endpoint에서 분할비율·신주배정·상장·거래정지 일정을 읽기 전용으로 수집한다.", True),
                ("접수번호 기반 근거 ID와 원 공시 URL을 직원·부장·본부장까지 추적한다.", True),
                ("기업행위 공식 근거가 없으면 역할·부서 보고의 근거 충족도를 35% 이하로 제한한다.", True),
                ("본부장 종합 수신 즉시 결과 체크포인트로 전환하고 화면 이동 애니메이션 완료 여부에 의존하지 않는다.", True),
                ("전문 부서 자동 소집으로 승격된 중요도는 복구 DB 체크포인트에도 important로 저장한다.", True),
                ("직원 근거 공백은 부서 보고 계약의 500자 상한으로 제한해 취합 실패를 막는다.", True),
                ("KRX 비공개 endpoint scraping이나 계약 없는 시장정보 재배포는 도입하지 않는다.", True),
            ],
        ),
        feature(
            "feat-user-triggered-codex-web-research",
            "사용자 요청형 Codex 공개 웹 조사",
            "별도 유료 검색 API 없이 사용자가 논문 연구를 지시한 시점에만 Codex 호스팅 웹 검색으로 공개 원문과 공식 자료를 조사한다.",
            "done",
            653,
            [
                ("논문 연구원을 직접 호출하거나 회의에서 해당 도구를 선택한 경우에만 웹 검색을 활성화한다.", True),
                ("일반 직원 세션은 웹 검색을 disabled로 유지하고 논문 조사 세션은 별도 thread key로 격리한다.", True),
                ("검색 세션도 read-only·shell network 차단·비밀 환경변수 제외 경계를 유지한다.", True),
                ("계좌·보유수량·현금·개인정보를 검색어에 넣지 않고 공개 주제만 검색하도록 강제한다.", True),
                ("회의 RoleReport 웹 근거는 codex-web-1~10, 전체 HTTPS URL과 관측 시각을 Rust·프런트에서 검증한다.", True),
                ("24시간 자동 수집이나 별도 유료 검색 API가 아니며 Codex 계정 사용량이 적용됨을 명시한다.", True),
            ],
        ),
        feature(
            "feat-department-report-detail-view",
            "부서별 장문 보고와 직원 근거 보존",
            "분석 기록에서 부서 보고를 독립 박스로 나누고 직원의 요약·세부 결과·근거·반대 근거·공백을 압축 손실 없이 표시한다.",
            "done",
            654,
            [
                ("각 부서 보고를 키보드로 접고 펼칠 수 있는 독립된 상세 박스로 표시한다.", True),
                ("부서 결론·근거 충족도·상세 종합을 큰 본문 글씨와 충분한 행간으로 표시한다.", True),
                ("직원 summary와 서로 다른 findings를 함께 부서장에게 전달하고 기록한다.", True),
                ("직원별 근거 ID·반대 근거·근거 공백과 부서 위험·후속 조치를 별도 구역으로 보존한다.", True),
                ("재무·뉴스 직원은 명시적 승인 후 격리된 웹 조사로 공식 원문을 보강할 수 있다.", True),
                ("근거 충족도는 공식성·최신성·교차검증·시점 정합성에 따라 평가하고 목표값을 강제하지 않는다.", True),
            ],
        ),
    ]


def normalize_flow(current):
    nodes = []
    for node in current["userFlow"]["nodes"]:
        metadata = json.loads(node.get("metadata_json") or "{}")
        nodes.append({
            "id": node["id"], "laneId": node["lane_id"], "title": node["title"], "description": node["description"], "kind": node["kind"],
            "positionX": node["position_x"], "positionY": node["position_y"], "colorKey": node.get("color_key") or "violet", "depth": node.get("depth"),
            "parentId": node.get("parent_id"), "linkedFeatureIds": json.loads(node.get("linked_feature_ids") or "[]"), "branchCondition": node.get("branch_condition"),
            "inputArtifacts": metadata.get("inputArtifacts", []), "outputArtifacts": metadata.get("outputArtifacts", []), "methods": metadata.get("methods", []),
            "validation": metadata.get("validation", ""), "failureHandling": metadata.get("failureHandling", ""), "codePaths": metadata.get("codePaths", []),
            "testPaths": metadata.get("testPaths", []), "completionCriteria": metadata.get("completionCriteria", ""), "isCompleted": bool(metadata.get("isCompleted", False)),
        })
    edges = [{"id": edge["id"], "sourceNodeId": edge["source_node_id"], "targetNodeId": edge["target_node_id"]} for edge in current["userFlow"]["edges"]]
    return nodes, edges


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("projectstudio_root", type=Path)
    args = parser.parse_args()
    api = load_api(args.projectstudio_root)
    database = api.default_database_path()
    with api.connect_database(database, writable=True) as connection:
        current = api.get_project(connection, PROJECT_ID)
        desired = desired_features()
        desired_by_id = {item["id"]: item for item in desired}
        features = [desired_by_id.get(item["id"], item) for item in current["features"]]
        existing_ids = {item["id"] for item in features}
        features.extend(item for item in desired if item["id"] not in existing_ids)
        nodes, edges = normalize_flow(current)
        node = next(item for item in nodes if item["id"] == FLOW_NODE_ID)
        node_before = copy.deepcopy(node)
        node["linkedFeatureIds"] = list(dict.fromkeys([*node["linkedFeatureIds"], *sorted(FEATURE_IDS)]))
        node["methods"] = list(dict.fromkeys([*node["methods"], "8개 전문 부서 직원별 독립 RoleReport", "실제 결과 기반 부장 취합", "Crossref 읽기 전용 메타데이터 브로커", "AgentToolPlan 역할별 도구 선택", "사용자 요청형 Codex 공개 웹 조사", "별도 웹 조사 thread 격리", "익명화 포지션·현금 도구", "비식별 운영·모의원장·감사·근거 manifest 도구", "도구 전후 이중 허용 목록 검증", "오프라인 Telegram 저장 근거 분리", "전 직원 독립 실행형 80회 중요 안건 상한", "OpenDART 회사분할 구조화 근거", "핵심 공식 근거 결측 신뢰도 상한", "직원·부서·종합 보고 5분·분류와 도구 선택 3분 제한", "부서 근거 공백 500자 상한", "정확한 evidenceId 허용 배열", "종합 수신 즉시 결과 체크포인트", "자동 승격 중요도 영속화", "직원 summary와 세부 findings 보존", "부서별 접기·펼치기 장문 보고", "재무·뉴스 공식 원문 웹 교차검증"]))
        node["codePaths"] = list(dict.fromkeys([*node["codePaths"], "src/employeeAgentOrchestration.ts", "src/App.tsx", "src-tauri/src/reference.rs", "src-tauri/src/codex.rs"]))
        node["testPaths"] = list(dict.fromkeys([*node["testPaths"], "scripts/employeeAgentOrchestration.test.ts", "src-tauri/src/reference.rs", "src-tauri/src/codex.rs"]))

        markdown = current["project"]["prd_markdown"]
        section = f"""## {SECTION_MARKER} (2026-09-03)
- 리서치부는 다섯 전문 직원이 각각 독립 RoleReport를 만들고 리서치 총괄이 실제 성공·실패 결과만 취합한다.
- 전략운용부의 Bull·Bear·트레이더·백테스트 연구원과 리스크관리부의 공격·중립·보수 위험·한도 감시·독립 모델검증도 각각 독립 RoleReport를 만들고 부장이 실제 결과만 취합한다.
- 매매운영·디지털자산·홍보·투자공학·준법감시 직원도 각각 독립 RoleReport를 만들며, 비식별 운영·모의원장·감사·근거 manifest 도구에서 확인 가능한 사실만 사용한다.
- 각 직원은 AgentToolPlan으로 역할별 허용 도구를 최대 3개 선택하고 Rust와 프런트 이중 검증 뒤 선택 결과만 두 번째 RoleReport turn에 전달받는다.
- 기술·재무공시·Telegram 뉴스·시장 레짐·익명화 포지션·현금·Crossref·명시적 GitHub 저장소 도구를 분리하며 주문·파일·명령·임의 URL 도구는 제공하지 않는다.
- Telegram 신규 동기화 실패 시 저장 근거는 cached_offline으로 표시하고 마지막 관측 시각을 보존하며 최신 뉴스로 표현하지 않는다.
- 직원별 실행 비용은 직원당 계획·보고 2회와 부장 취합 1회로 계산한다. 중요 회의는 전체 35명·8개 부서를 모두 실행할 수 있는 최대 80회 상한 안에서 실제 라우팅된 부서만 실행한다.
- 직원 역할 보고·부서 취합·본부장 종합은 high/xhigh reasoning과 긴 구조화 결과를 고려해 5분, 안건 분류·직원 도구 계획은 3분 제한을 두고, 초과 turn을 실제 중단한 뒤 성공으로 위장하지 않고 단계별 근거 공백 또는 hold로 닫는다.
- 직원 RoleReport에는 실제 도구가 반환한 evidenceId 허용 배열을 명시해 임시 ID와 `mv-*` 생성을 금지하고, 필요한 ID가 없으면 근거 공백으로만 보고한다.
- 직원 근거 공백은 부서장 입력 전에 500자 계약 상한으로 제한한다.
- 본부를 제외한 8개 전문 부서는 모두 employee_agent_v2 경로를 사용한다. 부서장은 실행되지 않은 직원을 대신해 roleFinding을 작성하지 않으며 예산 밖 부서는 명시적으로 제외한다.
- 국내 회사분할·신주·재상장 안건은 OpenDART 회사분할 결정 구조화 API를 추가 조회한다. 접수번호 기반 공식 근거가 없으면 역할·부서 근거 충족도를 35% 이하로 제한하고 재조회 조건을 표시한다.
- 본부장 종합 이벤트를 받으면 이동 애니메이션과 무관하게 즉시 결과 체크포인트로 전환하고, 전문 부서 소집으로 자동 승격된 중요도도 복구 DB에 저장한다.
- 논문 연구원을 직접 호출하거나 회의 Agent가 `research.codex_web_search`를 선택한 경우에만 별도 `paper-researcher-web` thread에서 Codex 호스팅 웹 검색을 활성화한다. 일반 직원은 웹 검색이 꺼진다.
- 웹 조사 세션도 read-only·shell network 차단·비밀 환경변수 제외를 유지하며, 계좌·보유수량·현금·개인정보를 검색어에 포함하지 않는다.
- 회의 RoleReport 웹 근거는 `codex-web-1`~`codex-web-10`, 전체 HTTPS URL과 관측 시각을 검증한다. 별도 유료 검색 API와 24시간 자동 수집은 사용하지 않고 Codex 계정 사용량만 적용한다.
"""
        header = f"## {SECTION_MARKER} (2026-09-03)"
        section_start = markdown.find(header)
        if section_start < 0:
            markdown += f"\n\n{section}"
        else:
            section_end = markdown.find("\n## ", section_start + len(header))
            if section_end < 0:
                section_end = len(markdown)
            markdown = f"{markdown[:section_start]}{section.rstrip()}{markdown[section_end:]}"
        if DETAIL_SECTION_MARKER not in markdown:
            markdown += f"""

## {DETAIL_SECTION_MARKER} (2026-09-04)
- 분석 기록은 부서별 보고를 하나의 짧은 목록으로 압축하지 않고 독립된 접기·펼치기 박스로 표시한다.
- 각 박스에는 부서 결론·근거 충족도·상세 종합, 직원별 결론·근거 ID·반대 근거·근거 공백, 부서 위험과 후속 조치를 함께 보존한다.
- 직원 RoleReport의 summary와 서로 다른 findings를 모두 부서장에게 전달하고, 부서·본부 단계의 텍스트 상한을 장문 보고가 손실되지 않는 범위로 확장한다.
- 내부 재무·공시·뉴스 공급자에 결측이 있으면 펀더멘털·뉴스 직원이 명시적 도구 계획과 격리된 읽기 전용 Codex 웹 조사를 거쳐 공식 원문을 교차 확인할 수 있다.
- 기업행위 공식 근거는 정확한 HTTPS 허용 호스트만 인정한다. 근거 충족도는 글 길이나 목표값이 아니라 공식성·최신성·교차검증·시점 정합성으로 평가한다.
"""

        current_by_id = {item["id"]: item for item in current["features"]}
        if all(current_by_id.get(item["id"]) == item for item in desired) and node_before == node and markdown == current["project"]["prd_markdown"]:
            print(json.dumps({"projectId": PROJECT_ID, "committed": False, "message": "동일한 직원별 Agent v2 기획이 이미 반영되어 있습니다."}, ensure_ascii=False, indent=2))
            return

        bundle = {
            "schemaVersion": 1,
            "projectId": PROJECT_ID,
            "expectedPrdRevisionNumber": current["project"]["revision_number"],
            "prd": {"title": current["project"]["prd_title"], "markdown": markdown},
            "features": features,
            "userFlow": {"nodes": nodes, "edges": edges},
        }
        api.validate_bundle(bundle)
        print(json.dumps(api.apply_bundle(connection, database, bundle, commit=True), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
