"""Telegram 원격운영 기반 기능을 ProjectStudio Investa 기획에 멱등 반영한다."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
from types import ModuleType
from typing import Any


PROJECT_ID = "36e87491-74a8-48ca-a7b8-30fa6ccea131"
SECTION_MARKER = "Telegram 원격운영과 Google Cloud·Gemini 연결"
IMPLEMENTATION_MARKER = "Cloud Run 원격운영 릴레이 구현 상태"
CLOUD_PROVISION_MARKER = "Google Cloud 원격운영 인프라 준비 상태"
ROUTING_DIAGNOSIS_MARKER = "Cloud Run 공개 URL 라우팅 진단"
POLICY_RECHECK_MARKER = "Cloud Run 정책·서비스 상태 재검사"


def load_api(projectstudio_root: Path) -> ModuleType:
    source = projectstudio_root / "scripts" / "projectstudio_api.py"
    spec = importlib.util.spec_from_file_location("projectstudio_api", source)
    if spec is None or spec.loader is None:
        raise RuntimeError("ProjectStudio 로컬 기획 API를 불러오지 못했습니다.")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def feature_nodes() -> list[dict[str, Any]]:
    return [
        {
            "id": "feat-operations-remote-control",
            "parentId": "req-operations",
            "title": "Telegram 원격운영 엔진",
            "description": "허용된 사용자의 자연어 지시를 분석·회의·모의주문 후보·자동매매 제어·시스템 제어로 분류하고 로컬 작업 큐와 승인 경계로 전달한다. 실제 주문은 열지 않는다.",
            "status": "in_progress",
            "priority": "high",
            "role": "사용자·운영자",
            "sortOrder": 2090,
            "colorKey": "green",
            "acceptanceCriteria": [
                {"id": "ac-remote-core", "description": "허용 사용자 검사, 명령 분류, SQLite 작업·사건 기록과 동일 요청 재전송 차단이 자동 테스트로 검증된다.", "isMet": True, "sortOrder": 0},
                {"id": "ac-remote-approval", "description": "투자·자동매매·시스템 제어 지시는 반드시 로컬 승인 대기로 저장되며 직접 주문하지 않는다.", "isMet": True, "sortOrder": 1},
                {"id": "ac-remote-provider", "description": "Telegram Bot, Google Cloud 릴레이와 Gemini 공급자를 계정 연결 후 실제 왕복 검증한다.", "isMet": False, "sortOrder": 2},
            ],
        },
        {
            "id": "feat-remote-command-contract",
            "parentId": "feat-operations-remote-control",
            "title": "원격 명령 분류·입력 검증",
            "description": "4,000자 제한, 제어문자·비밀정보 패턴 차단과 명령 종류의 결정론적 분류를 제공한다.",
            "status": "done",
            "priority": "high",
            "role": "원격운영 엔진",
            "sortOrder": 2091,
            "colorKey": "green",
            "acceptanceCriteria": [{"id": "ac-remote-contract-done", "description": "정상 명령과 변조·비밀정보 입력을 테스트한다.", "isMet": True, "sortOrder": 0}],
        },
        {
            "id": "feat-remote-identity-idempotency",
            "parentId": "feat-operations-remote-control",
            "title": "허용 사용자·중복 요청 차단",
            "description": "Telegram 사용자 ID allowlist와 source·request ID·내용 해시로 재전송을 멱등 처리한다.",
            "status": "done",
            "priority": "critical",
            "role": "보안 경계",
            "sortOrder": 2092,
            "colorKey": "green",
            "acceptanceCriteria": [{"id": "ac-remote-identity-done", "description": "미허용 사용자와 같은 ID의 변조 재전송을 거절한다.", "isMet": True, "sortOrder": 0}],
        },
        {
            "id": "feat-remote-job-ledger",
            "parentId": "feat-operations-remote-control",
            "title": "원격 작업·사건 SQLite 원장",
            "description": "수신, 큐 등록, 승인 필요와 승인·거절·취소 상태를 STRICT 테이블에 기록한다.",
            "status": "done",
            "priority": "high",
            "role": "운영 원장",
            "sortOrder": 2093,
            "colorKey": "green",
            "acceptanceCriteria": [{"id": "ac-remote-ledger-done", "description": "분석은 queued, 투자 지시는 awaiting_local_approval로 기록된다.", "isMet": True, "sortOrder": 0}],
        },
        {
            "id": "feat-remote-local-approval",
            "parentId": "feat-operations-remote-control",
            "title": "위험 명령 로컬 승인 게이트",
            "description": "모의주문 후보·섀도우·시스템 제어는 이 PC 사용자가 승인·거절·취소해야 다음 단계 후보가 된다.",
            "status": "done",
            "priority": "critical",
            "role": "사용자",
            "sortOrder": 2094,
            "colorKey": "green",
            "acceptanceCriteria": [{"id": "ac-remote-live-lock", "description": "승인해도 실전 주문은 계속 잠기며 기존 안전 게이트를 우회하지 않는다.", "isMet": True, "sortOrder": 0}],
        },
        {
            "id": "feat-remote-telegram-transport",
            "parentId": "feat-operations-remote-control",
            "title": "Telegram Bot 송수신 어댑터",
            "description": "Cloud Run에서 Telegram webhook secret과 numeric user allowlist를 검증해 Firestore 큐에 저장한다. 실제 Bot token·webhook 연결은 자격정보 입력 후 검증한다.",
            "status": "in_progress",
            "priority": "high",
            "role": "Telegram 어댑터",
            "sortOrder": 2095,
            "colorKey": "amber",
            "acceptanceCriteria": [
                {"id": "ac-remote-telegram-contract", "description": "webhook secret, 허용 사용자, 중복 update와 입력 한도를 자동 테스트한다.", "isMet": True, "sortOrder": 0},
                {"id": "ac-remote-telegram-live", "description": "실제 봇으로 지시 수신과 결과 회신을 왕복 검증한다.", "isMet": False, "sortOrder": 1},
            ],
        },
        {
            "id": "feat-remote-cloud-gemini-adapters",
            "parentId": "feat-operations-remote-control",
            "title": "Google Cloud 릴레이·Gemini 공급자",
            "description": "Cloud Run·Firestore relay와 데스크톱 HMAC 폴링, Google Cloud MFA·전용 프로젝트·Secret Manager·운영 리비전 적용을 완료했다. 공개 run.app 라우팅 404 해소와 Telegram 실제 왕복, Gemini 선택 공급자는 남아 있다.",
            "status": "in_progress",
            "priority": "high",
            "role": "클라우드·AI 어댑터",
            "sortOrder": 2096,
            "colorKey": "amber",
            "acceptanceCriteria": [
                {"id": "ac-remote-cloud-code", "description": "HMAC·nonce replay 차단, Firestore 멱등 큐·임대 복구와 desktop adapter 테스트가 통과한다.", "isMet": True, "sortOrder": 0},
                {"id": "ac-remote-cloud-account", "description": "MFA가 활성화된 전용 프로젝트에서 비용 제한과 실제 응답 회신을 검증한다.", "isMet": False, "sortOrder": 1},
                {"id": "ac-remote-gemini-provider", "description": "로컬 Codex와 분리된 Gemini 선택 공급자의 쿼터·타임아웃·비용 경계를 검증한다.", "isMet": False, "sortOrder": 2},
            ],
        },
        {
            "id": "feat-remote-cloud-relay-runtime",
            "parentId": "feat-remote-cloud-gemini-adapters",
            "title": "Cloud Run·Firestore 릴레이 런타임",
            "description": "Node 22 무의존성 서비스가 Telegram webhook을 멱등 저장하고 Firestore precondition 임대·만료 복구·결과 회신을 수행한다.",
            "status": "done",
            "priority": "high",
            "role": "클라우드 릴레이",
            "sortOrder": 2097,
            "colorKey": "green",
            "acceptanceCriteria": [{"id": "ac-relay-runtime-tested", "description": "서명 변조·stale timestamp·replay·미허용 사용자·로컬 승인 회신 테스트가 통과한다.", "isMet": True, "sortOrder": 0}],
        },
        {
            "id": "feat-remote-desktop-relay-adapter",
            "parentId": "feat-remote-cloud-gemini-adapters",
            "title": "데스크톱 Cloud relay 어댑터",
            "description": "HTTPS origin과 32바이트 secret을 Windows 자격 증명 관리자에 저장하고 15초 폴링 후 기존 SQLite 정책으로 재검증한다.",
            "status": "done",
            "priority": "high",
            "role": "데스크톱 어댑터",
            "sortOrder": 2098,
            "colorKey": "green",
            "acceptanceCriteria": [{"id": "ac-desktop-relay-tested", "description": "Rust·Node가 동일한 HMAC 계약을 사용하고 전체 Rust 테스트가 통과한다.", "isMet": True, "sortOrder": 0}],
        },
        {
            "id": "feat-remote-cloud-deployment",
            "parentId": "feat-remote-cloud-gemini-adapters",
            "title": "Google Cloud 운영 배포·비용 경계",
            "description": "MFA, 전용 프로젝트·필수 API·서울 Firestore, 최소 권한 서비스 계정, Secret Manager 3종, nonce TTL과 Cloud Run 최소 0·최대 1 리비전을 적용했다. run.app 공개 URL 404 때문에 webhook·실제 왕복과 예산 알림은 남아 있다.",
            "status": "in_progress",
            "priority": "high",
            "role": "클라우드 운영",
            "sortOrder": 2099,
            "colorKey": "amber",
            "acceptanceCriteria": [
                {"id": "ac-cloud-project-ready", "description": "전용 Google Cloud 프로젝트와 Cloud Run·Cloud Build·Artifact Registry·Firestore·Secret Manager API, 서울 Standard Firestore를 준비한다.", "isMet": True, "sortOrder": 0},
                {"id": "ac-cloud-runtime-applied", "description": "전용 서비스 계정·Secret 접근·Firestore TTL과 최소 0·최대 1 Cloud Run 리비전이 적용된다.", "isMet": True, "sortOrder": 1},
                {"id": "ac-cloud-deployment-live", "description": "공개 Cloud Run URL, Telegram webhook·desktop HMAC 실제 왕복과 비용 제한을 확인한다.", "isMet": False, "sortOrder": 2},
            ],
        },
        {
            "id": "feat-remote-gemini-provider",
            "parentId": "feat-remote-cloud-gemini-adapters",
            "title": "Gemini 선택형 분석 공급자",
            "description": "로컬 Codex 우선 경로와 분리해 사용자가 선택할 때만 Gemini API를 호출하고 모델·쿼터·비용 사용량을 기록한다.",
            "status": "planned",
            "priority": "medium",
            "role": "AI 공급자 어댑터",
            "sortOrder": 2100,
            "colorKey": "amber",
            "acceptanceCriteria": [{"id": "ac-gemini-live", "description": "별도 API 과금 동의와 키 연결 후 timeout·quota·응답 계약을 검증한다.", "isMet": False, "sortOrder": 0}],
        },
    ]


def normalize_existing_flow(current: dict[str, Any]) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    nodes: list[dict[str, Any]] = []
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


def append_user_flow(nodes: list[dict[str, Any]], edges: list[dict[str, str]]) -> None:
    if any(node["id"] == "flow-remote-phase" for node in nodes):
        return
    y = max((node["positionY"] for node in nodes), default=0) + 220
    common = {"laneId": "lane-remote-operations", "positionY": y, "colorKey": "green", "inputArtifacts": [], "outputArtifacts": [], "methods": [], "validation": "", "failureHandling": "", "codePaths": [], "testPaths": [], "completionCriteria": "", "isCompleted": False}
    definitions = [
        ("flow-remote-phase", "Telegram 원격운영", "외부에서 Investa에 자연어 업무를 지시하고 위험 명령은 로컬에서 재확인한다.", "phase", 10, 0, None, ["feat-operations-remote-control"], None),
        ("flow-remote-1", "Telegram에서 업무 지시", "분석, 부서 업무, 회의 또는 투자 운영 지시를 자연어로 보낸다.", "action", 170, 0, None, ["feat-remote-telegram-transport"], None),
        ("flow-remote-2", "사용자·재전송 검증", "등록 사용자와 요청 ID·내용 해시를 확인한다.", "decision", 430, 1, "flow-remote-1", ["feat-remote-identity-idempotency"], "미허용·변조 요청은 거절"),
        ("flow-remote-3", "명령 종류 분류", "분석·회의와 투자·자동매매·시스템 제어를 구분한다.", "decision", 690, 2, "flow-remote-2", ["feat-remote-command-contract"], "위험 명령 여부"),
        ("flow-remote-4", "로컬 승인 확인", "위험 명령이면 이 PC의 승인·거절을 기다린다.", "decision", 950, 3, "flow-remote-3", ["feat-remote-local-approval"], "분석은 바로 큐, 위험 명령은 승인 대기"),
        ("flow-remote-5", "Investa 작업 큐 실행", "연결된 Codex 또는 Gemini로 작업하고 기존 안전 게이트를 적용한다.", "action", 1210, 4, "flow-remote-4", ["feat-remote-job-ledger", "feat-remote-cloud-gemini-adapters"], None),
        ("flow-remote-6", "결과와 상태 회신", "근거·차단 사유·승인 대기 상태를 Telegram으로 회신한다.", "result", 1470, 5, "flow-remote-5", ["feat-remote-telegram-transport"], None),
    ]
    for node_id, title, description, kind, x, depth, parent, linked, branch in definitions:
        nodes.append({**common, "id": node_id, "title": title, "description": description, "kind": kind, "positionX": x, "depth": depth, "parentId": parent, "linkedFeatureIds": linked, "branchCondition": branch})
    chain = ["flow-remote-phase", "flow-remote-1", "flow-remote-2", "flow-remote-3", "flow-remote-4", "flow-remote-5", "flow-remote-6"]
    edges.extend({"id": f"edge-remote-{index}", "sourceNodeId": source, "targetNodeId": target} for index, (source, target) in enumerate(zip(chain, chain[1:])))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("projectstudio_root", type=Path)
    args = parser.parse_args()
    api = load_api(args.projectstudio_root)
    database = api.default_database_path()
    with api.connect_database(database, writable=True) as connection:
        current = api.get_project(connection, PROJECT_ID)
        desired_features = feature_nodes()
        replacements = {feature["id"]: feature for feature in desired_features}
        existing_by_id = {feature["id"]: feature for feature in current["features"]}
        comparison_keys = ("parentId", "title", "description", "status", "priority", "role", "sortOrder", "colorKey", "acceptanceCriteria")
        features_changed = any(
            feature["id"] not in existing_by_id
            or any(existing_by_id[feature["id"]].get(key) != feature.get(key) for key in comparison_keys)
            for feature in desired_features
        )
        features = [replacements.pop(feature["id"], feature) for feature in current["features"]]
        features.extend(replacements.values())
        nodes, edges = normalize_existing_flow(current)
        had_remote_flow = any(node["id"] == "flow-remote-phase" for node in nodes)
        append_user_flow(nodes, edges)
        markdown = current["project"]["prd_markdown"]
        had_section = SECTION_MARKER in markdown
        had_implementation_section = IMPLEMENTATION_MARKER in markdown
        had_cloud_provision_section = CLOUD_PROVISION_MARKER in markdown
        had_routing_diagnosis_section = ROUTING_DIAGNOSIS_MARKER in markdown
        had_policy_recheck_section = POLICY_RECHECK_MARKER in markdown
        if not had_section:
            markdown += f"""

## {SECTION_MARKER} (2026-08-25)
- Telegram은 종목 단축 명령 모음이 아니라 Investa 전체를 원격 운영하는 자연어 업무 채널로 사용한다.
- 원격 지시는 상태 조회, 분석, 회의, 모의주문 후보, 섀도우 제어와 시스템 제어로 분류한다.
- 허용 Telegram 사용자 ID만 처리하고 같은 업데이트 재전송은 멱등 처리한다.
- 분석·회의는 작업 큐에 등록하지만 투자·자동매매·시스템 제어는 이 PC의 승인을 기다린다. 승인 후에도 기존 위험 게이트와 주문 승인을 우회하지 않는다.
- 완료 범위는 Rust 명령 계약, SQLite 작업·사건 원장, allowlist, 중복 차단과 로컬 승인 상태 머신이다. Bot, Cloud 릴레이, Gemini 연결과 결과 회신은 계정 준비 후 연결한다.
- Google AI 구독과 Gemini API 비용은 별도 경계로 관리하고 Cloud 크레딧에도 예산 알림·쿼터·과금 상한을 적용한다.
"""
        if not had_implementation_section:
            markdown += f"""

## {IMPLEMENTATION_MARKER} (2026-08-25)
- 완료: Node 22 Cloud Run relay, Firestore 멱등 작업 큐·임대 만료 복구, Telegram webhook secret·허용 사용자 검증과 desktop HMAC·nonce replay 방지.
- 완료: Cloud relay 주소·장치 ID·공유 비밀값의 Windows 자격 증명 관리자 저장, 15초 폴링과 기존 SQLite allowlist·로컬 승인 게이트 재검증.
- 미완료: Google Cloud 계정 MFA 활성화, 전용 프로젝트·Firestore·Secret Manager·Cloud Run 실제 배포, Telegram Bot token·numeric user ID 왕복 검증.
- Gemini는 Google AI Pro 구독과 별도 API 과금 경계이므로 로컬 Codex 왕복 이후 선택 공급자로 연결한다.
"""
        if not had_cloud_provision_section:
            markdown += f"""

## {CLOUD_PROVISION_MARKER} (2026-08-26)
- Google Cloud 계정 MFA를 활성화하고 전용 프로젝트 `investa-remote-bumniverse`를 만들었다.
- Cloud Run, Cloud Build, Artifact Registry, Cloud Firestore와 Secret Manager API를 활성화했다.
- Firestore `(default)`를 Standard·기본 모드·기본 거부 보안 규칙·서울 `asia-northeast3` 리전으로 생성했다.
- 남은 작업은 전용 최소 권한 서비스 계정, Secret Manager 3종, nonce TTL, Cloud Run 최소 0·최대 1 배포, 예산 알림과 Telegram 실제 왕복이다.
- Telegram Bot token은 채팅·문서·SQLite에 기록하지 않고 Google Secret Manager에 사용자가 직접 등록한다.
"""
        if not had_routing_diagnosis_section:
            markdown += """

## Cloud Run 공개 URL 라우팅 진단 (2026-08-26)
- 완료: 전용 최소 권한 서비스 계정, Secret Manager 3종, Firestore nonce TTL과 Cloud Run 최소 0·최대 1 리비전을 운영 프로젝트에 적용했다.
- 완료: 운영 리비전은 Ready이며 Secret 원문은 문서·Git·SQLite·React·로그에 기록하지 않았다.
- 차단: 기존 서비스와 새 서비스의 공식 run.app URL이 모두 컨테이너 도달 전 HTTP 404를 반환한다. 동일 소스의 Cloud Shell 로컬 healthz는 HTTP 200이다.
- 안전 조치: 작동하지 않는 URL에는 Telegram webhook을 등록하지 않았고 실전 주문은 계속 잠겨 있다.
- 남은 완료 조건: 공개 healthz 200, Telegram 실제 수신, Firestore queue, desktop HMAC poll과 결과 회신 왕복, 예산 알림.
"""
        if not had_policy_recheck_section:
            markdown += """

## Cloud Run 정책·서비스 상태 재검사 (2026-08-26)
- 공개 상태판과 프로젝트별 Personalized Service Health에 서울 Cloud Run 활성 장애가 없다.
- 서비스의 전체 인터넷 ingress, 기본 HTTPS 엔드포인트와 공개 액세스가 모두 활성 상태다.
- 최근 2일간 HttpIngress 정책 거부 로그가 없고 프로젝트는 조직에 속하지 않아 사용자 정의 조직 정책·VPC Service Controls 가능성도 낮다.
- 따라서 남은 원인은 Google Cloud의 프로젝트별 run.app host mapping 또는 serving control-plane 이상으로 분류하고 지원 확인 전 webhook 등록을 보류한다.
"""
        if (
            not features_changed
            and had_remote_flow
            and had_section
            and had_implementation_section
            and had_cloud_provision_section
            and had_routing_diagnosis_section
            and had_policy_recheck_section
        ):
            print(json.dumps({"projectId": PROJECT_ID, "committed": False, "message": "이미 동일한 원격운영 기획이 반영되어 있습니다."}, ensure_ascii=False, indent=2))
            return
        bundle = {
            "schemaVersion": 1,
            "projectId": PROJECT_ID,
            "expectedPrdRevisionNumber": current["project"]["revision_number"],
            "prd": {"title": current["project"]["prd_title"], "markdown": markdown},
            "features": features,
            "userFlow": {"nodes": nodes, "edges": edges},
        }
        print(json.dumps(api.apply_bundle(connection, database, bundle, commit=True), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
