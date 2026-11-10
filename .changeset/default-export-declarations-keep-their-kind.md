---
monokl: patch
---

`export default function X() {}`, `export default async function X() {}`, `export default class X {}` and `export default interface X {}` are now ranked as a function, class or interface instead of as an unclassified export, so blocks such as axios `buildFullPath` and `dispatchRequest` rank higher. Default exports of expressions (`export default () => {}`, `export default X`) and `export { x as default }` keep the unclassified kind.
