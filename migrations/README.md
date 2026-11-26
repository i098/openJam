# Migrations

M0 owns `0001` to `0999`. Each later slice owns one range, so parallel pull requests never share a version number:

| Slice | Range |
|---|---|
| M1 tracking, M1b engine access | `1000` to `1999` |
| M2 analytics | `2000` to `2999` |
| M4 research and actions | `4000` to `4999` |
| M5 content and publishing | `5000` to `5999` |
| M6 agent surface | `6000` to `6999` |

Never edit a merged migration. Add a new one in your range.
