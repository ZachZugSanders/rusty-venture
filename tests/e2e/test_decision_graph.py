"""
Playwright end-to-end tests for the Decision-Graph view (Phase 3).

All backend API calls are intercepted by the ``mock_api`` fixture in
``conftest.py``, so no running Rust server is required.  The tests exercise
the 3D explorer, the filter panel, and the node inspector.
"""

import pytest
from playwright.sync_api import Page, expect


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def go_to_graph(page: Page, frontend_url: str) -> None:
    """Navigate to the frontend and switch to the Graph tab."""
    page.goto(frontend_url)
    # Click the "Graph" nav item
    page.get_by_role("button", name="Graph").click()
    expect(page.locator('[data-testid="decision-graph-view"]')).to_be_visible()


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


class TestDecisionGraphPageLoad:
    def test_graph_tab_is_present_in_nav(self, page: Page, frontend_url: str) -> None:
        """The nav bar must include a Graph tab."""
        page.goto(frontend_url)
        expect(page.get_by_role("button", name="Graph")).to_be_visible()

    def test_graph_view_renders_on_tab_click(
        self, page: Page, frontend_url: str
    ) -> None:
        """Clicking Graph tab shows the decision-graph view panel."""
        go_to_graph(page, frontend_url)
        expect(page.locator('[data-testid="decision-graph-view"]')).to_be_visible()

    def test_canvas_is_present(self, page: Page, frontend_url: str) -> None:
        """A Three.js <canvas> element must be rendered inside the graph view."""
        go_to_graph(page, frontend_url)
        expect(
            page.locator('[data-testid="decision-graph-view"] canvas')
        ).to_be_visible()

    def test_scan_selector_is_present(self, page: Page, frontend_url: str) -> None:
        """A scan selector control must be visible so users can pick a scan."""
        go_to_graph(page, frontend_url)
        expect(page.locator('[data-testid="scan-selector"]')).to_be_visible()


class TestDecisionGraphFilters:
    def test_filter_panel_is_present(self, page: Page, frontend_url: str) -> None:
        """A filters panel must be visible in the graph view."""
        go_to_graph(page, frontend_url)
        expect(page.locator('[data-testid="filter-panel"]')).to_be_visible()

    def test_filter_panel_has_kind_checkboxes(
        self, page: Page, frontend_url: str
    ) -> None:
        """The filter panel must contain checkboxes for each node kind."""
        go_to_graph(page, frontend_url)
        filter_panel = page.locator('[data-testid="filter-panel"]')
        expect(filter_panel.get_by_label("Root")).to_be_visible()
        expect(filter_panel.get_by_label("Dimension")).to_be_visible()
        expect(filter_panel.get_by_label("Signal")).to_be_visible()

    def test_highlight_only_toggle_is_present(
        self, page: Page, frontend_url: str
    ) -> None:
        """A 'Highlight only' toggle must be present in the filter panel."""
        go_to_graph(page, frontend_url)
        expect(
            page.locator('[data-testid="filter-panel"]').get_by_label("Highlight only")
        ).to_be_visible()

    def test_node_count_decreases_when_kind_unchecked(
        self, page: Page, frontend_url: str
    ) -> None:
        """Unchecking a node kind reduces the visible node count labels."""
        go_to_graph(page, frontend_url)

        # Wait for graph to finish loading (node-count must be > 0)
        node_count_el = page.locator('[data-testid="node-count"]')
        expect(node_count_el).not_to_have_text("0")

        count_before = int(node_count_el.inner_text())

        # Uncheck Signals
        page.locator('[data-testid="filter-panel"]').get_by_label("Signal").uncheck()

        count_after = int(node_count_el.inner_text())

        assert count_after < count_before, (
            f"Unchecking Signal should reduce node count "
            f"({count_before} → {count_after})"
        )

    def test_node_count_recovers_when_kind_rechecked(
        self, page: Page, frontend_url: str
    ) -> None:
        """Re-checking a kind restores the full node count."""
        go_to_graph(page, frontend_url)

        # Wait for graph to finish loading
        node_count_el = page.locator('[data-testid="node-count"]')
        expect(node_count_el).not_to_have_text("0")

        full_count = int(node_count_el.inner_text())

        signal_cb = page.locator('[data-testid="filter-panel"]').get_by_label("Signal")
        signal_cb.uncheck()
        signal_cb.check()

        expect(node_count_el).to_have_text(str(full_count))


class TestDecisionGraphInspector:
    def test_inspector_panel_is_present(self, page: Page, frontend_url: str) -> None:
        """An inspector panel must be visible beside the graph canvas."""
        go_to_graph(page, frontend_url)
        expect(page.locator('[data-testid="node-inspector"]')).to_be_visible()

    def test_inspector_shows_placeholder_when_no_node_selected(
        self, page: Page, frontend_url: str
    ) -> None:
        """When no node is selected the inspector shows a placeholder message."""
        go_to_graph(page, frontend_url)
        inspector = page.locator('[data-testid="node-inspector"]')
        expect(
            inspector.locator('[data-testid="inspector-placeholder"]')
        ).to_be_visible()

    def test_node_list_items_are_rendered(self, page: Page, frontend_url: str) -> None:
        """A list of node rows must be visible to allow non-click selection."""
        go_to_graph(page, frontend_url)
        items = page.locator('[data-testid="node-list-item"]')
        expect(items.first).to_be_visible()

    def test_selecting_node_from_list_updates_inspector(
        self, page: Page, frontend_url: str
    ) -> None:
        """Clicking a node-list item updates the inspector panel content."""
        go_to_graph(page, frontend_url)
        # Click the first node list item
        page.locator('[data-testid="node-list-item"]').first.click()

        inspector = page.locator('[data-testid="node-inspector"]')
        # Placeholder must disappear
        expect(
            inspector.locator('[data-testid="inspector-placeholder"]')
        ).not_to_be_visible()
        # Node detail must appear
        expect(inspector.locator('[data-testid="inspector-node-id"]')).to_be_visible()

    def test_inspector_shows_highlight_badge_for_failing_node(
        self, page: Page, frontend_url: str
    ) -> None:
        """Selecting a highlighted (failing) node shows the highlight badge."""
        go_to_graph(page, frontend_url)

        # Find and click the node list item whose id contains "has_security_policy"
        failing_item = page.locator(
            '[data-testid="node-list-item"][data-node-id="sig:Security:has_security_policy"]'
        )
        expect(failing_item).to_be_visible()
        failing_item.click()

        inspector = page.locator('[data-testid="node-inspector"]')
        expect(
            inspector.locator('[data-testid="inspector-highlight-badge"]')
        ).to_be_visible()
