"""OBIS Client 의 노선 데이터(data/routes/*.json, maps/*)를 komsi_obis 플러그인 폴더로 옮김.

플러그인은 자기 폴더 파일만 읽을 수 있어서, 노선 파일을 obis_routes/r<번호>.json 으로 복사하고
찾아보기용 index.json 을 만든다 (원본 Client 의 노선 맞추기와 같은 재료).

사용: python build_obis_routes.py [OBIS-client 폴더] [komsi_obis 폴더]
"""
import json, os, shutil, sys

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = sys.argv[1] if len(sys.argv) > 1 else r"C:\Program Files (x86)\Steam\steamapps\common\OMSI 2\OBIS-client"
DST = sys.argv[2] if len(sys.argv) > 2 else os.path.join(HERE, "..", "komsi_obis")


def norm_trip(name):
    # Client 의 _norm_trip_file: 경로 끝 이름, 소문자 (+ .ttp 는 노선 쪽에서 뗌)
    s = str(name or "").strip().replace("\\", "/").rsplit("/", 1)[-1].lower()
    return s[:-4] if s.endswith(".ttp") else s


def main():
    rdir = os.path.join(SRC, "data", "routes")
    out = os.path.join(DST, "obis_routes")
    if os.path.isdir(out):
        shutil.rmtree(out)
    os.makedirs(out)
    maps = sorted(d for d in os.listdir(os.path.join(SRC, "maps")) if os.path.isdir(os.path.join(SRC, "maps", d)))
    idx = {"default_map": maps[0] if maps else None, "routes": {}, "trips": {}}
    for n, fn in enumerate(sorted(os.listdir(rdir))):
        if not fn.endswith(".json"):
            continue
        raw = open(os.path.join(rdir, fn), encoding="utf-8").read()
        r = json.loads(raw)
        key = r.get("routeKey") or fn[:-5]
        file = f"r{n}.json"
        open(os.path.join(out, file), "w", encoding="utf-8").write(raw)
        idx["routes"][key] = {
            "file": file, "line": str(r.get("lineNumber") or ""), "map": r.get("mapId"),
            "stops": [int(s["id"]) for s in r.get("stations") or [] if s.get("id") is not None],
        }
        t = norm_trip(r.get("fileName"))
        if t:
            idx["trips"].setdefault(t, []).append(key)
    json.dump(idx, open(os.path.join(out, "index.json"), "w", encoding="utf-8"), ensure_ascii=False)
    print(f"{len(idx['routes'])} routes, {len(idx['trips'])} trip files, default map {idx['default_map']} -> {out}")


if __name__ == "__main__":
    main()
