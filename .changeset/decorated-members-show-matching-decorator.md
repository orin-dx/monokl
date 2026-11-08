---
monokl: patch
---

A `monokl search` row now shows a matching first line in `snippet` whenever the signature leaves part of it out. Before, a hit on `@HostListener('click')`, on `@Dec() prop = () => {` or on `load = () => needle(` left `snippet` null and the matching line appeared nowhere.
