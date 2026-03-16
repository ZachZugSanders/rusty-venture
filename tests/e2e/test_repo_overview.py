"""
Phase 2 — Playwright e2e tests: comprehensive repo overview panel.

When a repo row is clicked in the Repos view, the detail pane fetches
``GET /repos/:id/overview`` and ``GET /repos/:id/trends`` and renders four
information cards:

  1. **Posture** — composite score and league-tier grade badge.
  2. **Confidence** — percentage of signals that passed.
  3. **Top Blockers** — up to 5 failed signals with their point values.
  4. **Trend** — the historical composite-score data points.

All backend API calls are intercepted in conftest.py so no running Rust
server is required.  The stub data used here corresponds to a DIAMOND-grade
repo with 87.5 % confidence and three trend points.
"""

import pytest
from playwright.sync_api import Page, expect


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

_REPOS_VIEW_TIMEOUT = 5_000  # ms


def _open_repos_and_click_first(page: Page, frontend_url: str) -> None:
    """Navigate to the app and click the first repo row to open the detail pane."""
    page.goto(frontend_url)
    # Repos tab should be the default; wait for the repo table to load
    page.wait_for_selector("[data-testid='repo-row']", timeout=_REPOS_VIEW_TIMEOUT)
    page.locator("[data-testid='repo-row']").first.click()
    # Wait for the overview panel to appear
    page.wait_for_selector(
        "[data-testid='overview-panel']", timeout=_REPOS_VIEW_TIMEOUT
    )


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


class TestRepoOverviewPanel:
    """The repo detail pane shows a comprehensive overview when a row is selected."""

    def test_posture_grade_badge_is_displayed(
        self, page: Page, frontend_url: str
    ) -> None:
        """The posture card renders the league-tier grade with a coloured badge."""
        _open_repos_and_click_first(page, frontend_url)
        grade_badge = page.locator("[data-testid='overview-grade']")
        grade_badge.wait_for(state="visible", timeout=_REPOS_VIEW_TIMEOUT)
        # The stub returns DIAMOND — badge class should contain "green"
        class_attr = grade_badge.get_attribute("class") or ""
        assert (
            "green" in class_attr
        ), f"Posture badge expected class containing 'green', got: '{class_attr}'"
        assert grade_badge.inner_text().strip().upper() == "DIAMOND"

    def test_posture_composite_score_is_displayed(
        self, page: Page, frontend_url: str
    ) -> None:
        """The posture card shows the numeric composite score."""
        _open_repos_and_click_first(page, frontend_url)
        composite = page.locator("[data-testid='overview-composite']")
        composite.wait_for(state="visible", timeout=_REPOS_VIEW_TIMEOUT)
        # Stub returns composite = 92
        assert "92" in composite.inner_text()

    def test_confidence_percentage_is_displayed(
        self, page: Page, frontend_url: str
    ) -> None:
        """The confidence card shows the pass-rate percentage."""
        _open_repos_and_click_first(page, frontend_url)
        confidence = page.locator("[data-testid='overview-confidence']")
        confidence.wait_for(state="visible", timeout=_REPOS_VIEW_TIMEOUT)
        # Stub returns confidence = 87.5
        text = confidence.inner_text()
        assert "87" in text, f"Expected '87' in confidence text, got: '{text}'"

    def test_top_blockers_section_exists(self, page: Page, frontend_url: str) -> None:
        """The blockers card lists the failed signals."""
        _open_repos_and_click_first(page, frontend_url)
        # Stub has 2 blockers
        blockers = page.locator("[data-testid='blocker-item']")
        expect(blockers).to_have_count(2, timeout=_REPOS_VIEW_TIMEOUT)

    def test_top_blocker_detail_is_shown(self, page: Page, frontend_url: str) -> None:
        """Each blocker shows its signal name and detail text."""
        _open_repos_and_click_first(page, frontend_url)
        first_blocker = page.locator("[data-testid='blocker-item']").first
        first_blocker.wait_for(state="visible", timeout=_REPOS_VIEW_TIMEOUT)
        text = first_blocker.inner_text()
        assert (
            "no_critical_cves" in text or "critical" in text.lower()
        ), f"First blocker should mention the signal name or CVEs, got: '{text}'"

    def test_trend_panel_has_data_points(self, page: Page, frontend_url: str) -> None:
        """The trend section renders a list of historical score data points."""
        _open_repos_and_click_first(page, frontend_url)
        # Stub provides 3 trend points
        trend_points = page.locator("[data-testid='trend-point']")
        expect(trend_points).to_have_count(3, timeout=_REPOS_VIEW_TIMEOUT)

    def test_trend_points_show_composite_scores(
        self, page: Page, frontend_url: str
    ) -> None:
        """Each trend point displays its composite score."""
        _open_repos_and_click_first(page, frontend_url)
        points = page.locator("[data-testid='trend-point']").all()
        # Wait till they're there
        page.wait_for_selector(
            "[data-testid='trend-point']", timeout=_REPOS_VIEW_TIMEOUT
        )
        composites = [p.inner_text() for p in points]
        assert any("70" in t for t in composites), "Oldest trend point (70) not found"
        assert any("92" in t for t in composites), "Newest trend point (92) not found"
