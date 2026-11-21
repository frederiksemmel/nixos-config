# epaper

A bot on semmeltv3 that draws on the Waveshare 2.7" e-Paper HAT. The panel shows four shades of grey.

## Commands

| Command | What the panel shows |
|---|---|
| `/text <message>` | exactly `<message>`, as large as fits, lines balanced |
| `/ring` | the hand |
| `/clear` | white |
| `/rotate [0\|90\|180\|270]` | the same picture, turned; with no angle, 90° further |
| `/help` | nothing; the bot lists the commands |
| `/style [name\|off]` | nothing; the chat's replies switch to a style, such as `office`, or back with `off`. With no name, the bot names the current style and the others. |

Commands work in any letter case (`/Text` is `/text`). An unknown `/command` gets the help.

## Text

The message is Markdown. Every line of it stays a line on the panel.

| Write | Panel |
|---|---|
| `*schräg*` or `_schräg_` | italic |
| `**wichtig**` | white on black |
| `~~nöd~~` | struck through |
| `` `19:30` `` | monospace |
| `# Titel`, `## Titel` | 1.5× and 1.25× the size of the rest |
| `- eis`, `1. eis` | a bullet or number per line |

Emoji come from Noto Emoji, a black-and-white outline font. Nothing in a message is run as Typst code; markdown.rs turns it into a tree of strings and style names that text.typ draws.

## Who it listens to

- **WhatsApp:** only the chats listed in `whatsapp-groups`; the bot ignores every other chat.
  - A message that is exactly `hoi` (any case) adds its chat to the list, from anyone, and the bot greets. `ciao bello guys` (or `/stop`) removes the chat again, and the bot says goodbye.
  - In a private chat, every message is meant for the bot, as on Telegram.
  - In a group, only commands draw; ordinary chat stays off the panel. A message that names `spades` or replies to the bot counts as meant for it, so `spades hoi` draws `hoi`.
  - Messages older than 10 minutes, edits and the bot's own messages are skipped.
- **Telegram:** every message is meant for the bot. Plain text draws as if it came after `/text`. `services.epaper.allow` limits who may draw.

## Replies

A drawing command gets two messages: a wait line when the bot starts, and the outcome once the panel is done (written, busy, or failed). On WhatsApp the outcome is a new message; on Telegram it replaces the wait line. While the panel is drawing, other drawing commands get "busy" and are not queued.

The lines are picked at random from `replies.txt`. That file is data, not code; edit it on the Pi and the bot rereads it. Emoji reactions to the bot's lines are logged in `replies.jsonl`. Liked lines come up more often.

A style is a set of lines under sections like `[office wait]` next to the usual `[wait]`; every style in the file is one `/style` knows. A chat in a style gets the style's lines, and the usual lines for kinds the style has none of. `[office styled]` is said when a chat switches to `office`; `{style}` in it becomes the name. The chat's style is logged in `replies.jsonl`, so it survives restarts.

## Panel

- A drawing uses the **grey** refresh: the panel flashes black and white five times, then shows four shades (white, light grey, dark grey, black), so text edges and emoji come out smooth. It is slow on purpose, about 6 s, and leaves no ghosts.
- `/clear` uses the **clean** refresh: black and white only, flashing, about 2.7 s.
- The **fast** refresh drives only the pixels that change, in about 0.8 s without flashing, but in black and white and with faint ghosts. The bot doesn't use it; `--refresh fast` does.
- The controller takes one byte per SPI chip select; multi-byte writes are silently garbled.

## Rotation

`/rotate` saves the angle in `rotation`, so it survives restarts and deploys. `services.epaper.rotation` only applies while that file is missing.

## Files in /var/lib/epaper

Not in git; they hold chat ids and the WhatsApp session.

| File | Holds |
|---|---|
| `rotation` | the angle from the last `/rotate` |
| `replies.txt` | reply lines, by `[kind]` |
| `replies.jsonl` | sent lines and reactions |
| `whatsapp.db` | the linked-device session |
| `whatsapp-groups` | chats allowed to draw, one JID per line; `hoi` and `ciao bello guys` edit it |
| `whatsapp-qr`, `whatsapp-phone` | pairing: the current QR payload, or the number to ask a pair code for |

## Trying it without the bot

The bot holds the panel's pins, so stop it first (`sudo systemctl stop epaper`). Then, as the `epaper` user:

```
epaper --rotation 270 text "Hoi"
epaper --refresh clean clear
epaper --preview out.ppm text "Hoi"   # writes an image, leaves the panel alone
```
