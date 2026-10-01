"""Check the generated website's local links, headings, and platform downloads."""

import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit

from package import TARGETS


class Page(HTMLParser):
    def __init__(self, source):
        super().__init__()
        self.ids = set()
        self.links = []
        self.headings = 0
        self.packages = []
        self.feed(source)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if "data-package" in attrs:
            self.packages.append(attrs["data-package"])
        if "id" in attrs:
            if attrs["id"] in self.ids:
                raise ValueError(f"Duplicate ID: {attrs['id']}")
            self.ids.add(attrs["id"])
        if tag == "h1":
            self.headings += 1
        for key in ("src", "href"):
            if attrs.get(key):
                self.links.append(attrs[key])


def check_site(root):
    root = Path(root).resolve()
    pages = {p: Page(p.read_text(encoding="utf-8")) for p in root.rglob("*.html")}
    if root / "index.html" not in pages:
        raise ValueError("Missing index.html")
    packages = {f"merged_lands-{platform}.zip" for platform in TARGETS}
    for path, page in pages.items():
        if page.headings != 1:
            raise ValueError(f"{path.name}: expected exactly one h1")
        if len(page.packages) != len(packages) or set(page.packages) != packages:
            raise ValueError(f"{path.name}: download buttons do not match the platform archives")
        for link in page.links:
            url = urlsplit(link)
            if url.scheme or url.netloc:
                continue
            if url.path.startswith("/"):
                raise ValueError(f"{path.name}: root-relative link breaks project subpaths: {link}")
            target = (path.parent / unquote(url.path)).resolve() if url.path else path
            if target.is_dir():
                target = target / "index.html"
            if not target.is_relative_to(root) or not target.is_file():
                raise ValueError(f"{path.name}: missing local resource: {link}")
            if url.path.endswith(".html"):
                raise ValueError(f"{path.name}: use a clean URL instead of {link}")
            if url.fragment and target in pages and unquote(url.fragment) not in pages[target].ids:
                raise ValueError(f"{path.name}: missing anchor: {link}")
    print(f"Checked {len(pages)} pages: local links, assets, anchors, IDs, and all six platforms are valid.")


if __name__ == "__main__":
    check_site(sys.argv[1])
