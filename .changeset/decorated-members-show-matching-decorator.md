---
monokl: patch
---

A `monokl search` row for a decorated class member now shows a matching decorator line in `snippet`. Before, a hit on `@HostListener('click')` or on `@Dec() prop = () => {` left `snippet` null and the matching line appeared nowhere.
