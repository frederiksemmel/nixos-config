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
| `/mode [name]` | nothing; the chat's replies switch to a mode, such as `office`, or back with `swiss`. With no name, the bot names the chat's mode and the others. |

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

A drawing command gets two replies: a wait reply when the bot starts, and the outcome once the panel is done. On WhatsApp the outcome is a new message; on Telegram it replaces the wait reply. While the panel is drawing, other drawing commands get "busy" and are not queued.

Three words describe what the bot says:

- A **message** is what a reply has to get across, whatever the words.
- A **variant** is one wording of a message. Each reply is a variant picked at random.
- A **mode** is a set of variants in one voice. Each chat has one mode; `/mode` switches it.

| Message | Said when | Placeholder |
|---|---|---|
| `wait` | a drawing starts |  |
| `written` | the text is on the panel |  |
| `ring` | the hand is on the panel |  |
| `rotated` | the picture is turned | `{deg}`, the new angle |
| `busy` | the panel is still drawing; the command is dropped |  |
| `failed` | the panel broke; the journal says why |  |
| `switched` | the chat just switched to this mode | `{mode}`, its name |

| Mode | Voice |
|---|---|
| `swiss` | Swiss German memes, and the default for every chat |
| `office` | The Office (US), lines as said in the show |

Each mode is a file in `/var/lib/epaper/modes/`, such as `swiss.txt`. In it, `[message]` starts a section, and every following line is one variant until the next section; `#` starts a comment. A new file is a new mode. The files are data, not code: edit them on the Pi, and the bot rereads them when one changes.

A message that a mode has no variants of is said in a `swiss` variant. Emoji reactions to the bot's replies are logged in `replies.jsonl`, and liked variants come up more often. A variant is scored by its exact text, so rewording it starts its score from zero. Each chat's mode is logged there as well, so it survives restarts.

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
| `modes/<mode>.txt` | the variants of every message in that mode |
| `replies.jsonl` | the variants sent, reactions to them, and each chat's mode |
| `whatsapp.db` | the linked-device session |
| `whatsapp-groups` | chats allowed to draw, one JID per line; `hoi` and `ciao bello guys` edit it |
| `whatsapp-qr`, `whatsapp-phone` | pairing: the current QR payload, or the number to ask a pair code for |

## Trying it without the bot

The bot holds the panel's pins, so stop it first (`sudo systemctl stop epaper`). Then, as the `epaper` user:

```
epaper --rotation 270 text "Hoi"
epaper --refresh clean clear
epaper --preview out.ppm text "Hoi"   # writes an image, leaves the panel alone
epaper --preview out.ppm picture comic.png
```
