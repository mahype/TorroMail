#!/usr/bin/env python3
"""Create the single-current-release Sparkle feed shipped as a release asset."""

import html
import os
import re
import sys
from pathlib import Path
from xml.etree import ElementTree as ET

SPARKLE_NS = "http://www.andymatuschak.org/xml-namespaces/sparkle"
ET.register_namespace("sparkle", SPARKLE_NS)


def required(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        sys.exit(f"error: {name} is required")
    return value


def inline_markdown(text: str) -> str:
    text = html.escape(text, quote=False)
    text = re.sub(r"\*\*([^*]+)\*\*", r"<strong>\1</strong>", text)
    text = re.sub(r"`([^`]+)`", r"<code>\1</code>", text)
    return re.sub(
        r"\[([^\]]+)\]\((https?://[^)\s]+)\)",
        r'<a href="\2">\1</a>',
        text,
    )


def markdown_to_html(markdown: str) -> str:
    output: list[str] = []
    in_list = False

    def close_list() -> None:
        nonlocal in_list
        if in_list:
            output.append("</ul>")
            in_list = False

    for raw_line in markdown.splitlines():
        line = raw_line.rstrip()
        if not line.strip():
            close_list()
            continue
        heading = re.match(r"(#{1,6})\s+(.*)", line)
        if heading:
            close_list()
            level = min(len(heading.group(1)) + 1, 6)
            output.append(f"<h{level}>{inline_markdown(heading.group(2))}</h{level}>")
            continue
        bullet = re.match(r"\s*[-*]\s+(.*)", line)
        if bullet:
            if not in_list:
                output.append("<ul>")
                in_list = True
            output.append(f"<li>{inline_markdown(bullet.group(1))}</li>")
            continue
        close_list()
        output.append(f"<p>{inline_markdown(line)}</p>")
    close_list()
    return "\n".join(output)


def main() -> None:
    output_path = Path(required("OUTPUT_PATH"))
    version = required("VERSION")
    notes_url = required("RELEASE_NOTES_URL")
    notes_markdown = os.environ.get("RELEASE_NOTES_MD", "").strip()

    rss = ET.Element("rss", {"version": "2.0"})
    channel = ET.SubElement(rss, "channel")
    ET.SubElement(channel, "title").text = "TorroMail Updates"
    ET.SubElement(channel, "link").text = (
        "https://github.com/mahype/TorroMail/releases/latest/download/appcast.xml"
    )
    ET.SubElement(channel, "description").text = "Signed TorroMail releases"
    ET.SubElement(channel, "language").text = "en"

    item = ET.SubElement(channel, "item")
    ET.SubElement(item, "title").text = f"Version {version}"
    ET.SubElement(item, "pubDate").text = required("PUB_DATE")
    ET.SubElement(item, f"{{{SPARKLE_NS}}}shortVersionString").text = version
    ET.SubElement(item, f"{{{SPARKLE_NS}}}version").text = version
    ET.SubElement(item, f"{{{SPARKLE_NS}}}minimumSystemVersion").text = "14.0"
    if notes_markdown:
        ET.SubElement(item, "description").text = markdown_to_html(notes_markdown)
    else:
        ET.SubElement(item, f"{{{SPARKLE_NS}}}releaseNotesLink").text = notes_url
    ET.SubElement(
        item,
        "enclosure",
        {
            "url": required("DMG_URL"),
            f"{{{SPARKLE_NS}}}edSignature": required("DMG_ED_SIGNATURE"),
            "length": required("DMG_LENGTH"),
            "type": "application/octet-stream",
        },
    )

    output_path.parent.mkdir(parents=True, exist_ok=True)
    tree = ET.ElementTree(rss)
    ET.indent(tree, space="  ")
    tree.write(output_path, encoding="UTF-8", xml_declaration=True)


if __name__ == "__main__":
    main()
