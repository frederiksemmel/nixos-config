# epaper

A bot on semmeltv3 that draws on the Waveshare 2.7" e-Paper HAT. The panel shows four shades of grey.

## Commands

| Command | What the panel shows |
|---|---|
| `/text <message>` | exactly `<message>`, as large as fits, lines balanced |
| `/ring` | the hand |
| `/clear` | white |
| `/rotate [0\|90\|180\|270]` | the same picture, turned; with no angle, 90° further |
| `/xkcd [random\|<number>]` | the newest [xkcd](https://xkcd.com) comic, a random one, or that one; the reply carries its number, title and punchline |
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

## Stickers and pictures

A sticker sent to the bot goes on the panel, with the same wait and outcome replies as a command.

- **WhatsApp:** in a private chat, every sticker. In a group, a sticker that replies to one of the bot's messages; any other sticker stays chat. An animated sticker draws its first frame.
- **Telegram:** a still sticker draws as it is. An animated or video sticker draws its still thumbnail; without one, the bot answers that it can only draw still stickers.

A sticker and a text from the same person, sent within 10 s of each other in either order, share the panel: the sticker in the first half of the long side, the text fitted into the other half. The second one doesn't get "busy" while the first is still drawing; it waits and then draws both. In a WhatsApp group the second one counts as meant for the bot even if it doesn't reply to it, so a sticker that replies to the bot can be followed by plain text.

A picture is fitted into the panel at its current rotation, keeping its shape, and centred on white; transparent parts are white. How it is turned into four shades depends on the picture:

- **Line art**, such as a comic: nearly every pixel grey and close to black or white. It is darkened a little, so thin lines survive the scaling, and each pixel takes the nearest shade. Lines stay solid.
- **Everything else**, such as a sticker or a photo: the contrast is stretched to the full range, the midtones are lifted, and the shades are dithered (Atkinson), so gradients come out as fine dot patterns.

## GIFs

A GIF sent to the bot plays on the panel in black and white, with the same wait and outcome replies as a command. Who may send one is the same as for stickers:

- **WhatsApp:** in a private chat, every GIF. In a group, a GIF that replies to one of the bot's messages.
- **Telegram:** every GIF, which Telegram sends as an MP4 animation, and a GIF sent as a file.

The clip plays in its own time for at most 15 s; a shorter one plays once. The panel can't keep up with a GIF's pace, so it skips the pictures it is too slow for. Playing goes like this:

1. A clean refresh draws the first picture.
2. Each later picture is a fast refresh of only the window that changed, while the panel stays powered.
3. Once the clip is over, a grey refresh draws the first picture again in four shades, so no ghosts stay.

The panel is busy from the first picture to the last refresh. ffmpeg turns an MP4 into ten pictures a second, and a GIF is read directly. Each picture is fitted like a sticker and dithered to black and white with a fixed 4×4 pattern (ordered dithering), so a still area dithers the same in every picture and only what moves is redrawn.

## xkcd

`/xkcd` reads the comic from xkcd's JSON API and draws its picture as above. Some comics are interactive and have no PNG or JPEG picture; for those, and for numbers with no comic (such as 404), the bot says so and leaves the panel alone. So it does when xkcd can't be reached.

## Who it listens to

- **WhatsApp:** only the chats listed in `whatsapp-groups`; the bot ignores every other chat.
  - A message that is exactly `hoi` (any case) adds its chat to the list, from anyone, and the bot greets. `ciao bello guys` (or `/stop`) removes the chat again, and the bot says goodbye.
  - In a private chat, every message is meant for the bot, as on Telegram.
  - In a group, only commands draw; ordinary chat stays off the panel. A message that names `spades` or replies to the bot counts as meant for it, so `spades hoi` draws `hoi`. A sticker or GIF counts when it replies to the bot. A text or sticker that completes a sticker-and-text pair counts too.
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
| `written` | the text or picture is on the panel; after an xkcd, its number, title and punchline follow the variant |  |
| `ring` | the hand is on the panel |  |
| `rotated` | the picture is turned | `{deg}`, the new angle |
| `busy` | the panel is still drawing; the command is dropped |  |
| `failed` | the panel broke, or the picture could not be fetched or read; the journal says why |  |
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
- The **fast** refresh drives only the pixels that change, in about 0.8 s without flashing, but in black and white and with faint ghosts. Drawings don't use it; `--refresh fast` does, and so do GIFs.
- The controller takes one byte per SPI chip select; multi-byte writes are silently garbled.

## Animation speed

The time a refresh takes is mostly its waveform. Its length is fixed by the LUT rows and the frame rate the PLL register (0x30) sets, from the IL91874 datasheet:

| Refresh | Panel frames | Frame rate | Waveform |
|---|---|---|---|
| grey | 260 | 50.09 Hz | 5.2 s |
| clean | 152 | 81.8 Hz | 1.86 s |
| fast | 26 | 81.8 Hz | 0.32 s |

A fast refresh on its own takes about 0.8 s, because the panel is reset, powered on and off, and sent both pictures each time. A GIF skips that: the panel stays powered between pictures and gets only the changed window. On this controller that is commands 0x14 and 0x15 (old and new pixels for a window) and 0x16 (refresh that window). It has no 0x90 to 0x92 partial window commands. X and width are multiples of 8. Outside the window, the pixels follow the VCOM waveform and stay as they are. The whole panel is still scanned for every refresh, so a small window saves SPI transfer, not waveform time.

`epaper gif` measures it. `--frames N` shows N pictures in order with none skipped, `--lut-frames` shortens the fast waveform (25 by default), and `--full` sends the whole panel instead of the window. With `RUST_LOG=epaper=debug`, every picture logs its window, transfer and refresh time, and every run logs its fps. These timings have not yet been measured on the panel.

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
epaper --preview out.ppm xkcd random    # prints the number, title and punchline
epaper --rotation 270 gif spinner.gif   # plays as the bot does, logs the fps
epaper --rotation 270 gif --frames 20 --lut-frames 10 clip.mp4
```
