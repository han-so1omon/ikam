from fastapi.testclient import TestClient

from ikam_perf_report.main import app


def test_cors_allows_local_graph_viewer():
    client = TestClient(app)
    resp = client.get("/graph/summary", headers={"Origin": "http://localhost:5179"})
    assert resp.status_code == 200
    assert resp.headers.get("access-control-allow-origin") == "http://localhost:5179"


def test_cors_allows_compose_graph_viewer():
    client = TestClient(app)
    resp = client.get("/graph/summary", headers={"Origin": "http://ikam-graph-viewer:5179"})
    assert resp.status_code == 200
    assert resp.headers.get("access-control-allow-origin") == "http://ikam-graph-viewer:5179"


def test_cors_allows_worktree_graph_viewer_ports():
    client = TestClient(app)

    localhost_resp = client.get("/graph/summary", headers={"Origin": "http://localhost:5180"})
    loopback_resp = client.get("/graph/summary", headers={"Origin": "http://127.0.0.1:5180"})
    localhost_alt_resp = client.get("/graph/summary", headers={"Origin": "http://localhost:5181"})
    loopback_alt_resp = client.get("/graph/summary", headers={"Origin": "http://127.0.0.1:5181"})

    assert localhost_resp.status_code == 200
    assert loopback_resp.status_code == 200
    assert localhost_alt_resp.status_code == 200
    assert loopback_alt_resp.status_code == 200
    assert localhost_resp.headers.get("access-control-allow-origin") == "http://localhost:5180"
    assert loopback_resp.headers.get("access-control-allow-origin") == "http://127.0.0.1:5180"
    assert localhost_alt_resp.headers.get("access-control-allow-origin") == "http://localhost:5181"
    assert loopback_alt_resp.headers.get("access-control-allow-origin") == "http://127.0.0.1:5181"


def test_cors_exposes_content_disposition():
    client = TestClient(app)
    resp = client.get("/graph/summary", headers={"Origin": "http://localhost:5179"})
    assert resp.status_code == 200
    exposed = resp.headers.get("access-control-expose-headers", "")
    assert "content-disposition" in exposed.lower()
