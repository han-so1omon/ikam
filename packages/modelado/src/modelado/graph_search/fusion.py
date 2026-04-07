def fuse_candidates(content, relations, edges):
    merged = {}
    for bucket in (content, relations, edges):
        for item in bucket:
            fragment_id = item["fragment_id"]
            merged.setdefault(fragment_id, {"fragment_id": fragment_id, "evidence": []})
            merged[fragment_id]["evidence"].extend(item.get("evidence", []))
    return merged
