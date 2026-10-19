#!/usr/bin/env python3
"""Builds the UI preview fixture from a library dump.

    cargo run -q -p lumen-core --example dump_library -- ~/Vídeos/Lumen \\
      | python3 scripts/ui-fixture.py > ui/src/lib/mock/library.json

Skool keeps ONLY the real structure (courses, sections, lessons per section,
which lessons have a video or a description, resource kinds); every group,
course, section, lesson and resource name is replaced by deterministic,
invented text, and resource link targets by a placeholder, so regenerating the
fixture can never reintroduce names from a private library. A small synthetic
group is appended so the group switcher can be previewed.

Hotmart keeps ONLY the real structure (modules per course, lessons per
module, which lessons have no video, how many extra videos, materials and
readings); every name, seller and title is invented, so no purchase data
leaves the machine. Two DRM materials are added because the real library has
none and the UI must show them.
"""
import json
import sys

import re

PLACEHOLDER_URL = "https://example.com/resource"

SKOOL_GROUP_NAMES = ["ACADEMIA AURORA", "Colectivo Brújula", "Taller Marea"]
SKOOL_COURSE_TITLES = [
    "COMIENZA TU RECORRIDO", "HÁBITOS DE LOS QUE AVANZAN", "CONOCE LAS HERRAMIENTAS BÁSICAS",
    "LA MESA DE EDICIÓN", "EL ARTE DE ORDENAR PDFs", "EXPERIMENTOS UX.AI", "FaroIA",
    "Laboratorio de Diseño Aplicado a Proyectos Creativos", "Compón con DAW´s", "Modo Cabaña",
    "Guía Rápida UX",
]
SKOOL_SECTION_TITLES = [
    "Fundamentos", "Práctica guiada", "Proyecto final", "Clases en vivo", "Recursos", "Casos reales",
    "Preguntas y respuestas", "Laboratorio", "Revisión", "Para seguir avanzando", "Bonus", "Ejercicios",
]
SKOOL_LESSON_TITLES = [
    "Qué vas a aprender", "Herramientas necesarias", "Primer ejercicio", "Cómo organizar tu espacio",
    "Errores frecuentes", "Ejemplo paso a paso", "Revisión de entregas", "Plantilla de trabajo",
    "Ideas para practicar", "Resumen de la semana", "Preparar el material", "Rutina de enfoque",
    "Trabajo con referencias", "Ajustes finos", "Presentar tu proyecto", "Preguntas de la comunidad",
    "Lista de verificación", "Caso práctico", "Siguiente paso", "Repaso general",
    "Atajos útiles", "Planificar la sesión", "Mejorar con feedback", "Cierre del módulo",
]

FAKE_COURSES = [
    ("Fotografía para Emprendedores", "Estudio Norte"),
    ("Marketing de Contenidos Avanzado", "Academia Faro"),
    ("Finanzas Personales desde Cero", "Escuela Prisma"),
    ("Oratoria y Presentaciones", None),
]
TOPICS = [
    "Bienvenida", "Fundamentos", "Planificación", "Herramientas", "Primeros pasos", "Estrategia",
    "Contenido", "Audiencia", "Producción", "Edición", "Distribución", "Métricas", "Casos reales",
    "Automatización", "Ventas", "Comunidad", "Escalado", "Cierre", "Bonus", "Preguntas frecuentes",
]
LESSONS = [
    "Introducción", "Conceptos clave", "Ejercicio guiado", "Ejemplo práctico", "Errores comunes",
    "Checklist", "Plantilla de trabajo", "Caso de estudio", "Resumen", "Siguiente paso",
]


def scrub(library):
    """Anonymises every real Skool group; the synthetic ones are appended later."""
    platforms = {p["id"]: p for p in library.get("platforms", [])}
    skool = platforms.get("skool", {"id": "skool", "groups": library.get("groups", [])})
    skool["groups"] = [anonymise_group(g, i) for i, g in enumerate(skool["groups"])]
    return library


def leading_symbol(title):
    """An emoji-like prefix is kept so the UI keeps exercising emoji stripping."""
    m = re.match(r"^([^\w\s]+\s)", title)
    return m.group(1) if m else ""


def anonymise_group(group, gi):
    name = SKOOL_GROUP_NAMES[gi % len(SKOOL_GROUP_NAMES)]
    sec_n = 0
    les_n = 0
    courses = []
    for ci, course in enumerate(group["courses"], start=1):
        ctitle = SKOOL_COURSE_TITLES[(ci - 1) % len(SKOOL_COURSE_TITLES)]
        if ci > len(SKOOL_COURSE_TITLES):
            ctitle = f"{ctitle} {ci}"
        sections = []
        for si, section in enumerate(course["sections"], start=1):
            if section["title"] == course["title"]:
                stitle = ctitle
            else:
                stitle = SKOOL_SECTION_TITLES[sec_n % len(SKOOL_SECTION_TITLES)]
                if sec_n >= len(SKOOL_SECTION_TITLES):
                    stitle += f" {sec_n // len(SKOOL_SECTION_TITLES) + 1}"
                sec_n += 1
            lessons = []
            for lesson in section["lessons"]:
                base_title = SKOOL_LESSON_TITLES[les_n % len(SKOOL_LESSON_TITLES)]
                if les_n >= len(SKOOL_LESSON_TITLES):
                    base_title += f" {les_n // len(SKOOL_LESSON_TITLES) + 1}"
                les_n += 1
                ltitle = leading_symbol(lesson["title"]) + base_title
                li = lesson["index"]
                ldir = f"Skool/{name}/{ci:02d} - {ctitle}/{si:02d} - {stitle}/{li:02d} - {ltitle}"
                resources = []
                for n, res in enumerate(lesson["resources"], start=1):
                    ext = res["path"].rsplit(".", 1)[-1]
                    rname = f"Material de apoyo {n}"
                    resources.append({
                        "name": rname,
                        "path": f"{ldir}/{rname}.{ext}",
                        "url": PLACEHOLDER_URL if res.get("url") else None,
                        "kind": res["kind"],
                    })
                lessons.append({
                    "key": f"{ci}-{ctitle}/{si}-{stitle}/{li}-{ltitle}",
                    "title": ltitle,
                    "index": li,
                    "video": f"{ldir}/{ltitle}.mp4" if lesson["video"] else None,
                    "description": f"{ldir}/description.html" if lesson["description"] else None,
                    "hasVideo": lesson["hasVideo"],
                    "resources": resources,
                })
            sections.append({"title": stitle, "index": section["index"], "lessons": lessons})
        courses.append({"title": ctitle, "index": course["index"], "sections": sections})
    return {**group, "id": name, "name": name, "title": name, "courses": courses}


def synthetic_skool_group():
    def lesson(ci, ct, si, st, li, title, video=True):
        key = f"{ci}-{ct}/{si}-{st}/{li}-{title}"
        base = f"Skool/Estudio/{ci:02d} - {ct}/{si:02d} - {st}/{li:02d} - {title}"
        return {
            "key": key,
            "title": title,
            "index": li,
            "video": f"{base}/{title}.mp4" if video else None,
            "description": f"{base}/description.html",
            "hasVideo": video,
            "resources": [],
        }

    plan = [
        ("Fundamentos de la luz", [("Exposición", 4), ("Contraste", 3)]),
        ("Composición editorial", [("Retícula", 5)]),
        ("Color y tono", [("Paletas", 3), ("Gradación", 2)]),
    ]
    courses = []
    for ci, (ct, sections) in enumerate(plan, start=1):
        secs = []
        for si, (st, count) in enumerate(sections, start=1):
            lessons = [lesson(ci, ct, si, st, li, f"Lección {li}", video=li != count or si != 2) for li in range(1, count + 1)]
            secs.append({"title": st, "index": si, "lessons": lessons})
        courses.append({"title": ct, "index": ci, "sections": secs})
    count = sum(len(s["lessons"]) for c in courses for s in c["sections"])
    return {
        "id": "Estudio",
        "platform": "skool",
        "name": "Estudio",
        "title": "Estudio",
        "seller": None,
        "status": "partial",
        "lessonCount": count,
        "courses": courses,
    }


def synthetic_hotmart(real_groups):
    groups = []
    for gi, real in enumerate(real_groups[: len(FAKE_COURSES)]):
        title, seller = FAKE_COURSES[gi]
        folder = f"{title} - {seller}" if seller else title
        sections = []
        real_sections = real["courses"][0]["sections"]
        for si, rs in enumerate(real_sections, start=1):
            module = f"{TOPICS[(si - 1) % len(TOPICS)]}"
            mdir = f"{si}. {module}"
            lessons = []
            for li, rl in enumerate(rs["lessons"], start=1):
                quiz = not rl["hasVideo"]
                ltitle = "Responde al test" if quiz else LESSONS[(li - 1) % len(LESSONS)]
                ldir = f"Hotmart/{folder}/{mdir}/{li}. {ltitle}"
                resources = []
                kinds = [r["kind"] for r in rl["resources"]]
                for n, kind in enumerate(kinds, start=1):
                    if kind == "video":
                        resources.append({"name": f"{n + 1}. {ltitle}.mp4", "path": f"{ldir}/{n + 1}. {ltitle}.mp4", "url": None, "kind": "video"})
                    elif kind == "reading":
                        resources.append({"name": "Supplementary reading.html", "path": f"{ldir}/Supplementary reading.html", "url": None, "kind": "reading"})
                    elif kind == "link":
                        resources.append({"name": f"Enlace {n}.txt", "path": f"{ldir}/Enlace {n}.txt", "url": PLACEHOLDER_URL, "kind": "link"})
                    else:
                        resources.append({"name": f"Material {n}.pdf", "path": f"{ldir}/Materials/Material {n}.pdf", "url": None, "kind": "file"})
                lessons.append({
                    "key": f"hotmart:1-{title}/{si}-{module}/{li}-{ltitle}",
                    "title": ltitle,
                    "index": li,
                    "video": None if quiz else f"{ldir}/1. {ltitle}.mp4",
                    "description": f"{ldir}/Description.html",
                    "hasVideo": not quiz,
                    "resources": resources,
                })
            sections.append({"title": module, "index": si, "lessons": lessons})
        groups.append({
            "id": f"hotmart/{folder}",
            "platform": "hotmart",
            "name": folder,
            "title": title,
            "seller": seller,
            "status": real["status"],
            "lessonCount": sum(len(s["lessons"]) for s in sections),
            "courses": [{"title": title, "index": 1, "sections": sections}],
        })
    # DRM materials (none in the real library; the UI needs an example).
    first = groups[1]["courses"][0]["sections"][1]["lessons"][0] if len(groups) > 1 else None
    if first:
        base = first["description"].rsplit("/", 1)[0]
        for name in ("Guía de estudio.pdf", "Cuaderno de ejercicios.pdf"):
            first["resources"].append({"name": name, "path": f"{base}/Materials/drm_{name}", "url": None, "kind": "drm"})
    return groups


def main():
    library = scrub(json.load(sys.stdin))
    platforms = {p["id"]: p for p in library.get("platforms", [])}
    skool = platforms.get("skool", {"id": "skool", "groups": library.get("groups", [])})
    skool["groups"].append(synthetic_skool_group())
    real_hotmart = platforms.get("hotmart", {"groups": []})["groups"]
    out = {
        "platforms": [skool, {"id": "hotmart", "groups": synthetic_hotmart(real_hotmart)}],
        "groups": skool["groups"],
    }
    json.dump(out, sys.stdout, ensure_ascii=False, indent=1)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
