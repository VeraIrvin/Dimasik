#!/usr/bin/env python3
"""Generate per-province 1897 district GeoJSON from the official RiStat GeoPackages.

Source: RISTAT 1897 administrative reconstruction, DataverseNL dataset
https://doi.org/10.34894/NQOASN (download bundle:
https://dataverse.nl/api/access/datafiles/578935,578936). The dataset record
grants CC0 with an attribution request; ristat.org separately states
CC BY-NC-SA 4.0, so cite the deposit ("Russian Empire 1897" / ristat.org)
wherever this data is displayed.

Usage (from the repository root):

    python3 scripts/generate-uyezds-1897.py \
        "/tmp/ristat-1897/Russian Empire 1897 - Districts" \
        "/tmp/ristat-1897/Russian Empire 1897 - Provinces"

Both input files are verified against their published SHA1 values before any
feature is read. Only the Python standard library is used; no GIS runtime.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sqlite3
import struct
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

DISTRICTS_SHA1 = "679a760e7bbab3a25974e55f7fa3f6858f65506c"
PROVINCES_SHA1 = "cb2ebb7e346500e670bcf39c079fd2258dcebe3d"
EXPECTED_CANONICAL_PROVINCES = 76

# Explicit source-name variants. The expected RiStat Gub_ID makes every alias
# fail closed if a future source revision changes which feature it identifies.
# RiStat itself describes IDs 62 and 70 as the full province including the
# named округа; those are the appropriate province rows for their districts.
SOURCE_NAME_ALIASES: dict[str, tuple[str, int]] = {
    "Тифлисская губерния": ("Тифлисская губерния вкл. Закатальский округ", 70),
    "Петербургская губерния": ("Санкт-Петербургская губерния", 2),
    "Абосская губерния": ("Або-Бьернеборгская губерния", 5),
    "Кутаисская губерния": ("Кутаисская  губерния вкл. Сухумский округ", 62),
    "Елизаветопольская губерния": ("Елисаветпольская губерния", 66),
    # The canonical source uses the gubernial capital (Ревель); RiStat uses
    # the historical territorial name (Эстляндия) for the same governorate.
    "Ревельская губерния": ("Эстляндская губерния", 19),
}

DISTRICT_COLUMNS = (
    "fid",
    "geom",
    "Distr_ID",
    "Gub_ID",
    "Name_RU",
    "Name_ENG",
    "prov_RU",
    "prov_ENG",
    "RISTAT_ID",
)
PROVINCE_COLUMNS = (
    "fid",
    "geom",
    "Gub_ID",
    "prov_RU",
    "prov_ENG",
    "RISTAT_ID",
)


class SourceError(ValueError):
    """The input data does not meet the verified RiStat data contract."""


def sha1(path: Path) -> str:
    digest = hashlib.sha1()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_checksum(path: Path, expected: str) -> None:
    actual = sha1(path)
    if actual != expected:
        raise SourceError(f"SHA1 mismatch for {path}: expected {expected}, got {actual}")


def table_columns(connection: sqlite3.Connection, table: str) -> tuple[str, ...]:
    quoted = table.replace('"', '""')
    return tuple(row[1] for row in connection.execute(f'PRAGMA table_info("{quoted}")'))


def require_geopackage_schema(
    connection: sqlite3.Connection,
    table: str,
    expected_columns: tuple[str, ...],
) -> None:
    columns = table_columns(connection, table)
    if columns != expected_columns:
        raise SourceError(
            f"Unexpected {table} columns: expected {expected_columns!r}, got {columns!r}"
        )
    geometry_metadata = [
        tuple(row)
        for row in connection.execute(
            "SELECT column_name, geometry_type_name, srs_id, z, m "
            "FROM gpkg_geometry_columns WHERE table_name = ?",
            (table,),
        )
    ]
    expected_metadata = [("geom", "MULTIPOLYGON", 4326, 0, 0)]
    if geometry_metadata != expected_metadata:
        raise SourceError(
            f"Unexpected {table} geometry metadata: {geometry_metadata!r}"
        )
    srs_row = connection.execute(
        "SELECT organization, organization_coordsys_id "
        "FROM gpkg_spatial_ref_sys WHERE srs_id = 4326"
    ).fetchone()
    srs = tuple(srs_row) if srs_row is not None else None
    if srs != ("EPSG", 4326):
        raise SourceError(f"Unexpected EPSG:4326 metadata in {table}: {srs!r}")


class WkbReader:
    def __init__(self, data: bytes):
        self.data = data
        self.offset = 0

    def take(self, size: int) -> bytes:
        end = self.offset + size
        if end > len(self.data):
            raise SourceError("Truncated WKB geometry")
        value = self.data[self.offset:end]
        self.offset = end
        return value

    def unpack(self, byte_order: str, format_code: str) -> Any:
        size = struct.calcsize(format_code)
        return struct.unpack(byte_order + format_code, self.take(size))[0]

    def geometry(self) -> dict[str, Any]:
        byte_order_marker = self.unpack("<", "B")
        if byte_order_marker == 0:
            byte_order = ">"
        elif byte_order_marker == 1:
            byte_order = "<"
        else:
            raise SourceError(f"Invalid WKB byte-order marker: {byte_order_marker}")

        raw_type = self.unpack(byte_order, "I")
        if raw_type & 0xE0000000:
            raise SourceError(f"EWKB flags are not expected in this GeoPackage: {raw_type:#x}")
        dimensions, geometry_type = divmod(raw_type, 1000)
        if dimensions != 0:
            raise SourceError(f"Only two-dimensional WKB is supported, got type {raw_type}")

        if geometry_type == 3:
            return {"type": "Polygon", "coordinates": self.polygon_body(byte_order)}
        if geometry_type == 6:
            count = self.unpack(byte_order, "I")
            polygons = []
            for _ in range(count):
                child = self.geometry()
                if child["type"] != "Polygon":
                    raise SourceError("MultiPolygon contains a non-Polygon child")
                polygons.append(child["coordinates"])
            if not polygons:
                raise SourceError("Empty MultiPolygon is not valid source geometry")
            return {"type": "MultiPolygon", "coordinates": polygons}
        raise SourceError(f"Unsupported WKB geometry type: {geometry_type}")

    def polygon_body(self, byte_order: str) -> list[list[list[float]]]:
        ring_count = self.unpack(byte_order, "I")
        if ring_count == 0:
            raise SourceError("Empty Polygon is not valid source geometry")
        rings = []
        for _ in range(ring_count):
            point_count = self.unpack(byte_order, "I")
            if point_count < 4:
                raise SourceError(f"Polygon ring has only {point_count} points")
            ring = []
            for _ in range(point_count):
                x = self.unpack(byte_order, "d")
                y = self.unpack(byte_order, "d")
                if not math.isfinite(x) or not math.isfinite(y):
                    raise SourceError("Geometry contains a non-finite coordinate")
                ring.append([x, y])
            if ring[0] != ring[-1]:
                raise SourceError("Polygon ring is not closed")
            rings.append(ring)
        return rings


def decode_geopackage_geometry(blob: bytes | None) -> dict[str, Any]:
    if blob is None or len(blob) < 8:
        raise SourceError("Missing or truncated GeoPackage geometry")
    if blob[:2] != b"GP":
        raise SourceError("Invalid GeoPackage geometry magic")
    if blob[2] != 0:
        raise SourceError(f"Unsupported GeoPackage geometry version: {blob[2]}")

    flags = blob[3]
    if flags & 0xE0:
        raise SourceError(f"Reserved GeoPackage geometry flags are set: {flags:#x}")
    if flags & 0x10:
        raise SourceError("Extended GeoPackage geometries are not supported")
    if flags & 0x08:
        raise SourceError("Empty GeoPackage geometry is not valid source geometry")
    header_order = "<" if flags & 0x01 else ">"
    srs_id = struct.unpack(header_order + "i", blob[4:8])[0]
    if srs_id != 4326:
        raise SourceError(f"Geometry has SRS {srs_id}, expected EPSG:4326")

    envelope_code = (flags >> 1) & 0x07
    envelope_sizes = {0: 0, 1: 32, 2: 48, 3: 48, 4: 64}
    if envelope_code not in envelope_sizes:
        raise SourceError(f"Reserved GeoPackage envelope code: {envelope_code}")
    wkb_offset = 8 + envelope_sizes[envelope_code]
    reader = WkbReader(blob[wkb_offset:])
    geometry = reader.geometry()
    if reader.offset != len(reader.data):
        raise SourceError(
            f"Trailing bytes after WKB geometry: {len(reader.data) - reader.offset}"
        )
    if geometry["type"] not in {"Polygon", "MultiPolygon"}:
        raise SourceError(f"Unexpected decoded geometry: {geometry['type']}")
    return geometry


def load_canonical_provinces(path: Path) -> list[dict[str, str]]:
    try:
        collection = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SourceError(f"Cannot read canonical provinces from {path}: {error}") from error
    if collection.get("type") != "FeatureCollection":
        raise SourceError(f"Canonical file is not a FeatureCollection: {path}")
    provinces = []
    for feature in collection.get("features", []):
        properties = feature.get("properties", {})
        province_id = properties.get("id")
        name = properties.get("name")
        if not isinstance(province_id, str) or not province_id:
            raise SourceError(f"Invalid canonical province ID: {province_id!r}")
        if not isinstance(name, str) or not name:
            raise SourceError(f"Invalid canonical province name for {province_id}: {name!r}")
        if Path(province_id).name != province_id or province_id in {".", ".."}:
            raise SourceError(f"Unsafe canonical province ID for filename: {province_id!r}")
        provinces.append({"id": province_id, "name": name})
    if len(provinces) != EXPECTED_CANONICAL_PROVINCES:
        raise SourceError(
            f"Expected {EXPECTED_CANONICAL_PROVINCES} canonical provinces, got {len(provinces)}"
        )
    for field in ("id", "name"):
        duplicates = sorted(
            value for value, count in Counter(item[field] for item in provinces).items() if count > 1
        )
        if duplicates:
            raise SourceError(f"Duplicate canonical province {field}s: {duplicates!r}")
    return provinces


def require_unique(rows: list[sqlite3.Row], field: str, label: str) -> None:
    values = [row[field] for row in rows]
    missing = [index for index, value in enumerate(values) if value is None]
    if missing:
        raise SourceError(f"Missing {label} {field} at row indexes: {missing[:10]!r}")
    duplicates = sorted(value for value, count in Counter(values).items() if count > 1)
    if duplicates:
        raise SourceError(f"Duplicate {label} {field}: {duplicates[:10]!r}")


def open_source(path: Path) -> sqlite3.Connection:
    uri = f"file:{path.resolve().as_posix()}?mode=ro"
    connection = sqlite3.connect(uri, uri=True)
    connection.row_factory = sqlite3.Row
    return connection


def generate(
    districts_path: Path,
    provinces_path: Path,
    canonical_path: Path,
    output_directory: Path,
) -> dict[str, Any]:
    require_checksum(districts_path, DISTRICTS_SHA1)
    require_checksum(provinces_path, PROVINCES_SHA1)
    canonical = load_canonical_provinces(canonical_path)

    with open_source(districts_path) as districts_db, open_source(provinces_path) as provinces_db:
        require_geopackage_schema(districts_db, "districts_1897", DISTRICT_COLUMNS)
        require_geopackage_schema(provinces_db, "provinces_1897", PROVINCE_COLUMNS)
        districts = districts_db.execute(
            "SELECT fid, geom, Distr_ID, Gub_ID, Name_RU, prov_RU "
            "FROM districts_1897 ORDER BY Gub_ID, fid"
        ).fetchall()
        source_provinces = provinces_db.execute(
            "SELECT fid, geom, Gub_ID, prov_RU FROM provinces_1897 ORDER BY fid"
        ).fetchall()

    if len(districts) != 824 or len(source_provinces) != 103:
        raise SourceError(
            f"Unexpected source counts: {len(districts)} districts, "
            f"{len(source_provinces)} provinces"
        )
    require_unique(districts, "fid", "district")
    require_unique(districts, "Distr_ID", "district")
    require_unique(source_provinces, "fid", "province")
    require_unique(source_provinces, "Gub_ID", "province")

    provinces_by_name: dict[str, list[sqlite3.Row]] = defaultdict(list)
    provinces_by_numeric_id: dict[int, sqlite3.Row] = {}
    for row in source_provinces:
        provinces_by_name[row["prov_RU"]].append(row)
        try:
            numeric_id = int(row["Gub_ID"])
        except (TypeError, ValueError):
            continue
        if str(numeric_id) == str(row["Gub_ID"]):
            provinces_by_numeric_id[numeric_id] = row

    districts_by_province: dict[int, list[sqlite3.Row]] = defaultdict(list)
    for district in districts:
        province_id = district["Gub_ID"]
        if not isinstance(province_id, int):
            raise SourceError(f"District {district['fid']} has non-numeric Gub_ID {province_id!r}")
        province = provinces_by_numeric_id.get(province_id)
        if province is None:
            raise SourceError(
                f"District {district['fid']} references missing province Gub_ID {province_id}"
            )
        if district["prov_RU"] != province["prov_RU"]:
            raise SourceError(
                f"District {district['fid']} province name {district['prov_RU']!r} does not "
                f"match Gub_ID {province_id} name {province['prov_RU']!r}"
            )
        if not isinstance(district["Name_RU"], str) or not district["Name_RU"]:
            raise SourceError(f"District {district['fid']} has no Russian name")
        districts_by_province[province_id].append(district)

    joined: list[tuple[dict[str, str], sqlite3.Row]] = []
    used_source_ids: set[int] = set()
    for province in canonical:
        canonical_name = province["name"]
        source_name, expected_id = SOURCE_NAME_ALIASES.get(
            canonical_name, (canonical_name, None)
        )
        matches = provinces_by_name.get(source_name, [])
        if len(matches) != 1:
            raise SourceError(
                f"Canonical province {canonical_name!r} matched {len(matches)} RiStat rows "
                f"using source name {source_name!r}"
            )
        source = matches[0]
        source_id = int(source["Gub_ID"])
        if expected_id is not None and source_id != expected_id:
            raise SourceError(
                f"Alias {canonical_name!r} expected Gub_ID {expected_id}, got {source_id}"
            )
        if source_id in used_source_ids:
            raise SourceError(f"RiStat Gub_ID {source_id} was joined more than once")
        if not districts_by_province[source_id]:
            raise SourceError(f"Canonical province {canonical_name!r} has zero districts")
        used_source_ids.add(source_id)
        joined.append((province, source))

    if set(SOURCE_NAME_ALIASES) - {item["name"] for item in canonical}:
        raise SourceError("SOURCE_NAME_ALIASES contains an unused canonical name")

    output_directory.mkdir(parents=True, exist_ok=True)
    expected_files = {f"{province['id']}.geojson" for province, _ in joined}
    unexpected_files = {
        existing.name
        for existing in output_directory.glob("*.geojson")
        if existing.name not in expected_files
    }
    if unexpected_files:
        raise SourceError(f"Unexpected GeoJSON files in {output_directory}: {sorted(unexpected_files)!r}")

    generated_districts = 0
    sizes: dict[str, int] = {}
    for province, source in joined:
        source_id = int(source["Gub_ID"])
        features = [
            {
                "type": "Feature",
                "properties": {
                    "kind": "province",
                    "id": province["id"],
                    "name": province["name"],
                },
                "geometry": decode_geopackage_geometry(source["geom"]),
            }
        ]
        for district in sorted(districts_by_province[source_id], key=lambda row: row["fid"]):
            features.append(
                {
                    "type": "Feature",
                    "properties": {
                        "kind": "district",
                        "id": f"uyezd-1897-{district['fid']}",
                        "name": district["Name_RU"],
                        "provinceId": province["id"],
                    },
                    "geometry": decode_geopackage_geometry(district["geom"]),
                }
            )
            generated_districts += 1
        collection = {"type": "FeatureCollection", "features": features}
        destination = output_directory / f"{province['id']}.geojson"
        destination.write_text(
            json.dumps(collection, ensure_ascii=False, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        sizes[destination.name] = destination.stat().st_size

    actual_files = {path.name for path in output_directory.glob("*.geojson")}
    if actual_files != expected_files:
        raise SourceError(
            f"Output file set mismatch; missing={sorted(expected_files - actual_files)!r}, "
            f"extra={sorted(actual_files - expected_files)!r}"
        )
    largest_file = max(sizes, key=sizes.__getitem__)
    return {
        "files": len(sizes),
        "districts": generated_districts,
        "sourceDistricts": len(districts),
        "excludedSourceDistricts": len(districts) - generated_districts,
        "largestFile": largest_file,
        "largestFileBytes": sizes[largest_file],
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("districts", type=Path, help="RiStat 1897 districts GeoPackage")
    parser.add_argument("provinces", type=Path, help="RiStat 1897 provinces GeoPackage")
    parser.add_argument(
        "--canonical",
        type=Path,
        default=Path("public/data/gubernias.geojson"),
        help="canonical province FeatureCollection (default: %(default)s)",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("public/data/uyezds-1897"),
        help="output directory (default: %(default)s)",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    result = generate(args.districts, args.provinces, args.canonical, args.output)
    print(json.dumps(result, ensure_ascii=False, sort_keys=True))


if __name__ == "__main__":
    main()
