"""
Phase 4 — Playwright e2e regression tests: full user-workflow coverage.

These tests exercise complete navigation and data-display workflows to guard
against regressions across tab switches, API-driven renders, and UI state
transitions.  All backend calls are intercepted by the ``mock_api`` autouse
fixture in conftest.py so no running Rust server is required.
"""

import pytest
from playwright.sync_api import Page, expect


# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

_TIMEOUT = 8_000  # ms — generous for CI machines
_GRAPH_TIMEOUT = 12_000  # Three.js init needs a bit longer in headless mode


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _navigate(page: Page, frontend_url: str) -> None:
    """Go to the app root and wait for the nav bar to appear."""
    page.goto(frontend_url)
    page.wait_for_selector("nav button", timeout=_TIMEOUT)


def _click_tab(page: Page, label: str, timeout: int = _TIMEOUT) -> None:
    """Click a nav tab by its visible text label."""
    page.locator("nav button", has_text=label).click()
    page.wait_for_timeout(300)  # allow React state update


def _collect_page_errors(page: Page) -> list[str]:
    """Return page-level JS errors caught with ``page.on('pageerror')``."""
    errors: list[str] = []
    page.on("pageerror", lambda exc: errors.append(str(exc)))
    return errors


# ---------------------------------------------------------------------------
# Suite 1: all tabs load without JS errors
# ---------------------------------------------------------------------------


class TestAllViewsLoad:
    """Each navigation tab renders its primary content element without
    unhandled JS exceptions."""

    def test_repos_tab_renders_repo_rows(self, page: Page, frontend_url: str) -> None:
        errors = _collect_page_errors(page)
        _navigate(page, frontend_url)
        # Repos is the default tab — repo rows should appear immediately.
        page.wait_for_selector("[data-testid='repo-row']", timeout=_TIMEOUT)
        rows = page.locator("[data-testid='repo-row']")
        assert rows.count() > 0, "Repos tab must show at least one repo row"
        assert errors == [], f"JS errors on Repos tab: {errors}"

    def test_history_tab_renders_table(self, page: Page, frontend_url: str) -> None:
        errors = _collect_page_errors(page)
        _navigate(page, frontend_url)
        _click_tab(page, "History")
        # HistoryView renders a <table> when scans > 0.
        page.wait_for_selector("table", timeout=_TIMEOUT)
        assert page.locator("table").is_visible()
        assert errors == [], f"JS errors on History tab: {errors}"

    def test_analyze_tab_renders_form(self, page: Page, frontend_url: str) -> None:
        errors = _collect_page_errors(page)
        _navigate(page, frontend_url)
        _click_tab(page, "Analyze")
        # AnalyzeView renders a form with a URL input.
        page.wait_for_selector("input[type='url']", timeout=_TIMEOUT)
        assert page.locator("input[type='url']").is_visible()
        assert errors == [], f"JS errors on Analyze tab: {errors}"

    def test_graph_tab_renders_decision_graph_view(
        self, page: Page, frontend_url: str
    ) -> None:
        errors = _collect_page_errors(page)
        _navigate(page, frontend_url)
        _click_tab(page, "Graph")
        page.wait_for_selector(
            "[data-testid='decision-graph-view']", timeout=_GRAPH_TIMEOUT
        )
        assert page.locator("[data-testid='decision-graph-view']").is_visible()
        assert errors == [], f"JS errors on Graph tab: {errors}"


# ---------------------------------------------------------------------------
# Suite 2: Repos workflow — click row → overview panel
# ---------------------------------------------------------------------------


class TestReposWorkflow:
    """Clicking a repo row fetches and renders the overview panel."""

    def _open_overview(self, page: Page, frontend_url: str) -> None:
        _navigate(page, frontend_url)
        page.wait_for_selector("[data-testid='repo-row']", timeout=_TIMEOUT)
        page.locator("[data-testid='repo-row']").first.click()
        page.wait_for_selector("[data-testid='overview-panel']", timeout=_TIMEOUT)

    def test_overview_panel_appears_after_click(
        self, page: Page, frontend_url: str
    ) -> None:
        self._open_overview(page, frontend_url)
        assert page.locator("[data-testid='overview-panel']").is_visible()

    def test_overview_shows_grade_badge(self, page: Page, frontend_url: str) -> None:
        self._open_overview(page, frontend_url)
        grade = page.locator("[data-testid='overview-grade']")
        grade.wait_for(state="visible", timeout=_TIMEOUT)
        # Stub returns DIAMOND → badge colour class should contain "green"
        class_attr = grade.get_attribute("class") or ""
        assert (
            "green" in class_attr
        ), f"DIAMOND badge expected 'green' colour class, got: '{class_attr}'"

    def test_overview_shows_composite_score(
        self, page: Page, frontend_url: str
    ) -> None:
        self._open_overview(page, frontend_url)
        composite = page.locator("[data-testid='overview-composite']")
        composite.wait_for(state="visible", timeout=_TIMEOUT)
        # Stub composite = 92
        assert "92" in (
            composite.inner_text() or ""
        ), "Overview panel must display the composite score (stub: 92)"

    def test_overview_shows_confidence(self, page: Page, frontend_url: str) -> None:
        self._open_overview(page, frontend_url)
        confidence = page.locator("[data-testid='overview-confidence']")
        confidence.wait_for(state="visible", timeout=_TIMEOUT)
        # Stub confidence = 87.5 %
        text = confidence.inner_text() or ""
        assert "87" in text, f"Confidence value should contain '87', got: '{text}'"


# ---------------------------------------------------------------------------
# Suite 3: History tab content
# ---------------------------------------------------------------------------


class TestHistoryTabContent:
    """History tab renders scan rows with grade badges from the mocked API."""

    def test_history_scan_rows_are_present(self, page: Page, frontend_url: str) -> None:
        _navigate(page, frontend_url)
        _click_tab(page, "History")
        page.wait_for_selector("table tbody tr", timeout=_TIMEOUT)
        rows = page.locator("table tbody tr")
        # Stub provides 10 scans (one per grade)
        assert (
            rows.count() == 10
        ), f"Expected 10 history rows from stub, found {rows.count()}"

    def test_history_rows_contain_repo_url(self, page: Page, frontend_url: str) -> None:
        _navigate(page, frontend_url)
        _click_tab(page, "History")
        page.wait_for_selector("table tbody tr", timeout=_TIMEOUT)
        first_row = page.locator("table tbody tr").first
        text = first_row.inner_text()
        # Each stub row URL contains "example/repo-"
        assert (
            "example/repo-" in text
        ), f"First history row must contain a repo URL segment, got: '{text}'"


# ---------------------------------------------------------------------------
# Suite 4: Decision Graph workflow
# ---------------------------------------------------------------------------


class TestGraphWorkflow:
    """Decision Graph tab: scan selector, node list, and filter panel render
    correctly with the mocked graph payload."""

    def _open_graph(self, page: Page, frontend_url: str) -> None:
        _navigate(page, frontend_url)
        _click_tab(page, "Graph")
        page.wait_for_selector(
            "[data-testid='decision-graph-view']", timeout=_GRAPH_TIMEOUT
        )

    def test_scan_selector_is_populated(self, page: Page, frontend_url: str) -> None:
        self._open_graph(page, frontend_url)
        selector = page.locator("[data-testid='scan-selector']")
        selector.wait_for(state="visible", timeout=_TIMEOUT)
        # 10 stub scans → 10 <option> elements
        options = selector.locator("option")
        assert (
            options.count() == 10
        ), f"Scan selector should have 10 options from stub, found {options.count()}"

    def test_node_list_shows_nodes(self, page: Page, frontend_url: str) -> None:
        self._open_graph(page, frontend_url)
        # Wait for list items to appear (graph fetch completes)
        page.wait_for_selector("[data-testid='node-list-item']", timeout=_GRAPH_TIMEOUT)
        items = page.locator("[data-testid='node-list-item']")
        # Stub graph has 4 nodes; all visible by default (no filters applied)
        assert (
            items.count() == 4
        ), f"Node list should show 4 nodes from stub graph, found {items.count()}"

    def test_node_count_badge_matches_stub(self, page: Page, frontend_url: str) -> None:
        self._open_graph(page, frontend_url)
        count_span = page.locator("[data-testid='node-count']")
        count_span.wait_for(state="visible", timeout=_GRAPH_TIMEOUT)
        assert (
            count_span.inner_text().strip() == "4"
        ), f"Node count badge should read '4', got: '{count_span.inner_text()}'"

    def test_filter_panel_is_visible(self, page: Page, frontend_url: str) -> None:
        self._open_graph(page, frontend_url)
        fp = page.locator("[data-testid='filter-panel']")
        fp.wait_for(state="visible", timeout=_TIMEOUT)
        assert fp.is_visible()


# ---------------------------------------------------------------------------
# Suite 5: Tab switching — no state corruption
# ---------------------------------------------------------------------------


class TestTabSwitching:
    """Cycling through all four tabs should leave each in a clean state."""

    def test_full_tab_cycle_renders_without_errors(
        self, page: Page, frontend_url: str
    ) -> None:
        errors = _collect_page_errors(page)
        _navigate(page, frontend_url)

        for label, selector in [
            ("History", "table"),
            ("Analyze", "input[type='url']"),
            ("Graph", "[data-testid='decision-graph-view']"),
            ("Repos", "[data-testid='repo-row']"),
        ]:
            _click_tab(page, label)
            page.wait_for_selector(selector, timeout=_GRAPH_TIMEOUT)

        assert errors == [], f"JS errors during tab cycle: {errors}"

    def test_repos_remains_functional_after_tab_roundtrip(
        self, page: Page, frontend_url: str
    ) -> None:
        """After leaving and returning to Repos the row list re-renders."""
        _navigate(page, frontend_url)
        _click_tab(page, "History")
        _click_tab(page, "Repos")
        page.wait_for_selector("[data-testid='repo-row']", timeout=_TIMEOUT)
        assert page.locator("[data-testid='repo-row']").count() > 0
