# epaper

A bot on semmeltv3 that draws on the Waveshare 2.7" e-Paper HAT. The panel shows black and white only.

## Commands

| Command | What the panel shows |
|---|---|
| `/text <message>` | exactly `<message>`, as large as fits, lines balanced |
| `/ring` | the hand |
| `/clear` | white |
| `/rotate [0\|90\|180\|270]` | the same picture, turned; with no angle, 90° further |
| `/help` | nothing; the bot lists the commands |

Commands work in any letter case (`/Text` is `/text`). An unknown `/command` gets the help.

## Who it listens to

- **WhatsApp:** only the groups listed in `whatsapp-groups`; private chats and other groups are ignored. In a group, only commands draw; ordinary chat stays off the panel. A message that names `spades` or replies to the bot counts as meant for it, so `spades hoi` draws `hoi`. Messages older than 10 minutes, edits and the bot's own messages are skipped.
- **Telegram:** every message is meant for the bot. Plain text draws as if it came after `/text`. `services.epaper.allow` limits who may draw.

## Replies

A drawing command gets two messages: a wait line when the bot starts, and the outcome once the panel is done (written, busy, or failed). On WhatsApp the outcome is a new message; on Telegram it replaces the wait line. While the panel is drawing, other drawing commands get "busy" and are not queued.

The lines are picked at random from `replies.txt`. That file is data, not code; edit it on the Pi and the bot rereads it. Emoji reactions to the bot's lines are logged in `replies.jsonl`. Liked lines come up more often.

## Panel

- A drawing uses the **fast** refresh: only the pixels that change are driven, with no flashing. It takes about 0.8 s and leaves faint ghosts.
- `/clear`, and every 11th drawing, uses the **clean** refresh: the panel flashes black and white and the ghosts go away. It takes about 2.7 s.
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
| `whatsapp-groups` | group JIDs allowed to draw, one per line |
| `whatsapp-qr`, `whatsapp-phone` | pairing: the current QR payload, or the number to ask a pair code for |

## Trying it without the bot

The bot holds the panel's pins, so stop it first (`sudo systemctl stop epaper`). Then, as the `epaper` user:

```
epaper --rotation 270 text "Hoi"
epaper --refresh clean clear
epaper --preview out.ppm text "Hoi"   # writes an image, leaves the panel alone
```
