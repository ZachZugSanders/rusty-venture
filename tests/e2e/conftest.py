"""
Shared fixtures for Playwright e2e tier-badge tests.

A lightweight Python HTTP server serves the pre-built frontend bundle from
``frontend/dist/``.  All backend API calls are intercepted by ``page.route()``
so no running Rust server is required.
"""

import functools
import http.server
import json
import os
import threading

import pytest

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------

_REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
FRONTEND_DIST = os.path.join(_REPO_ROOT, "frontend", "dist")

_SERVER_PORT = 18_080


# ---------------------------------------------------------------------------
# Session-scoped static file server
# ---------------------------------------------------------------------------


@pytest.fixture(scope="session")
def frontend_server():
    """Start a one-shot HTTP server for the compiled frontend bundle."""
    handler = functools.partial(
        http.server.SimpleHTTPRequestHandler,
        directory=FRONTEND_DIST,
    )
    server = http.server.HTTPServer(("127.0.0.1", _SERVER_PORT), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    yield f"http://127.0.0.1:{_SERVER_PORT}"
    server.shutdown()


@pytest.fixture(scope="session")
def frontend_url(frontend_server: str) -> str:
    return frontend_server


# ---------------------------------------------------------------------------
# API stub helpers
# ---------------------------------------------------------------------------

_ALL_GRADES = [
    "DIAMOND",
    "PLATINUM",
    "GOLD",
    "SILVER",
    "BRONZE",
    # Legacy backward-compat values that may still be in the database
    "EXEMPLARY",
    "ESTABLISHED",
    "DEVELOPING",
    "EMERGING",
    "NASCENT",
]


def _scan_stub(grade: str, idx: int = 0) -> dict:
    return {
        "id": f"scan-{grade.lower()}-{idx}",
        "repo_url": f"https://github.com/example/repo-{grade.lower()}",
        "scanned_at": "2026-03-15T12:00:00Z",
        "duration_ms": 5_000,
        "risk_score": 25,
        "composite_maturity": 75,
        "maturity_grade": grade,
    }


def _repo_stub(grade: str, idx: int = 0) -> dict:
    return {
        "id": f"repo-{idx:02d}",
        "url": f"https://github.com/example/repo-{idx:02d}",
        "first_seen": "2026-03-15T10:00:00Z",
        "last_scanned": "2026-03-15T12:00:00Z",
        "scan_count": 1,
        "latest_maturity_grade": grade,
        "latest_composite_maturity": 75,
        "latest_risk_score": 25,
    }


def _overview_stub(repo_id: str = "repo-00", grade: str = "DIAMOND") -> dict:
    """Return a RepoOverview-shaped payload for mock API responses."""
    return {
        "repo_id": repo_id,
        "repo_url": f"https://github.com/example/{repo_id}",
        "composite": 92,
        "grade": grade,
        "confidence": 87.5,
        "dimensions": [
            {
                "dimension": "Security",
                "score": 95,
                "weight": 0.3,
                "passed_count": 7,
                "total_count": 8,
            },
            {
                "dimension": "Dependency Health",
                "score": 88,
                "weight": 0.2,
                "passed_count": 5,
                "total_count": 6,
            },
        ],
        "top_blockers": [
            {
                "signal_name": "no_critical_cves",
                "dimension": "Security",
                "points": 30,
                "detail": "3 critical CVEs found in dependencies",
            },
            {
                "signal_name": "has_security_policy",
                "dimension": "Security",
                "points": 15,
                "detail": "SECURITY.md is missing",
            },
        ],
    }


def _trends_stub(repo_id: str = "repo-00") -> list:
    """Return a list of TrendPoint-shaped payloads for mock API responses."""
    return [
        {
            "scanned_at": "2026-01-15T12:00:00Z",
            "composite": 70,
            "grade": "GOLD",
            "dimensions": {"Security": 72, "Dependency Health": 68},
        },
        {
            "scanned_at": "2026-02-15T12:00:00Z",
            "composite": 82,
            "grade": "PLATINUM",
            "dimensions": {"Security": 85, "Dependency Health": 78},
        },
        {
            "scanned_at": "2026-03-15T12:00:00Z",
            "composite": 92,
            "grade": "DIAMOND",
            "dimensions": {"Security": 95, "Dependency Health": 88},
        },
    ]


# ---------------------------------------------------------------------------
# Per-test API route mocks
# ---------------------------------------------------------------------------


@pytest.fixture(autouse=True)
def mock_api(page):
    """Intercept all backend API calls with deterministic stub data."""
    scans_payload = json.dumps(
        {
            "success": True,
            "data": [_scan_stub(g, i) for i, g in enumerate(_ALL_GRADES)],
        }
    )
    repos_payload = json.dumps(
        {
            "success": True,
            "data": [_repo_stub(g, i) for i, g in enumerate(_ALL_GRADES)],
        }
    )
    overview_payload = json.dumps({"success": True, "data": _overview_stub()})
    trends_payload = json.dumps({"success": True, "data": _trends_stub()})

    def handle_scans(route):
        route.fulfill(status=200, content_type="application/json", body=scans_payload)

    def handle_repos(route):
        route.fulfill(status=200, content_type="application/json", body=repos_payload)

    def handle_overview(route):
        route.fulfill(
            status=200, content_type="application/json", body=overview_payload
        )

    def handle_trends(route):
        route.fulfill(status=200, content_type="application/json", body=trends_payload)

    page.route("**/scans**", handle_scans)
    page.route("**/repos**", handle_repos)
    # Register more specific routes AFTER so they take precedence over **/repos**
    page.route("**/overview**", handle_overview)
    page.route("**/trends**", handle_trends)
