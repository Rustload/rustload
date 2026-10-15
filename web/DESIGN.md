# Rustload UI design notes

The brand itself (name, logo, palette, typography) is described in
[`docs/brand`](../docs/brand/README.md). These notes cover how the interface applies it.

Rustload is a tab people leave open on a second screen. The interface behaves like a measuring
instrument: calm, dense, legible at a glance, with one live readout that matters most.

## Principles

1. **Oxide means moving.** The signal colour (oxide in the light theme, ember in the dark one) is
   reserved for live transfers (speed bars, active progress, the dashboard readout) and the single
   primary action per screen; a primary action that can't run yet turns neutral. Status, selection
   and toggles use neutral ink, never the signal colour. The logo is the only other place it
   appears.
2. **Numbers are condensed, words are not.** Every figure (speeds, sizes, ETAs, counters) uses the
   `num` utility: Archivo at 78 % width with tabular figures, so live values never shift the layout.
   The hero readout uses `readout` (64 % width). Headings use `heading` (112 % width).
3. **Say it, then show it.** The dashboard opens with one plain sentence about what Rustload is
   doing ("Rustload is downloading at 84 MiB/s", "Everything is paused"), the figures that matter
   under it, then one panel per package on the move: its overall bar and percentage first, the
   files in progress as rows inside. Failures sit above everything else, with a retry button.
   Elsewhere lists are rows; panels only group things.
4. **Quiet chrome.** 1 px borders in `line`, radius 6 px on controls, 10 px on panels and 4 px on
   chips nested in them, a single pop-up shadow for menus/dialogs/toasts. No gradients, no blur.
5. **Show the machinery where it is read.** The dashboard keeps to plain, solid progress bars that
   read at a glance. The _segment map_ (100 slices of the file, oxide where data is on disk, a bright
   tick where each connection is writing) lives in the queue, next to each file in transfer. The
   sign-in screen plays the same idea once (`FillStrip`), then steps back.
6. **Check before committing.** Anything the user adds goes through the link grabber first: links
   light up as they are checked (lamp: hollow = checking/unknown, green = online, red = unavailable,
   gold = no account), grouped into the packages that will be created.
7. **Keyboard first.** <kbd>Ctrl</kbd>+<kbd>K</kbd> or <kbd>/</kbd> opens the command palette,
   <kbd>N</kbd> adds, <kbd>G</kbd> then a letter navigates, <kbd>?</kbd> lists shortcuts. Pasting
   links anywhere opens the grabber with them.

## Tokens (`src/app.css`)

| Token                          | Light (aluminium)                                 | Dark (steel)                                          | Use                                                 |
| ------------------------------ | ------------------------------------------------- | ----------------------------------------------------- | --------------------------------------------------- |
| `bg`                           | `#e8ecef`                                         | `#1c242c`                                             | page background                                     |
| `panel`                        | `#f5f7f8`                                         | `#222c35`                                             | panels, rows                                        |
| `sunken`                       | `#dfe4e8`                                         | `#182027`                                             | inputs, wells, progress tracks                      |
| `line`                         | `#cdd5dc`                                         | `#34414d`                                             | borders, dividers                                   |
| `fg` / `fg-muted` / `fg-faint` | `#18212a` / `#56636f` / `#7d8994`                 | `#e4e9ed` / `#93a1ad` / `#6b7a87`                     | text                                                |
| `signal`                       | `#c24e1b` oxide (ink `#a6420f`, text on it white) | `#f27318` ember (ink `#ff9150`, text on it `#1f0f04`) | live transfers, primary action                      |
| `brand`                        | `#c24e1b`                                         | `#f27318`                                             | the logo's mark                                     |
| `ok` / `bad` / `info` / `warn` | `#1f8a5b` / `#c22f55` / `#2f6db5` / `#8a6400`     | `#45c08e` / `#f2738a` / `#79aee9` / `#dcb862`         | completed / failed / remote work & focus / warnings |
| `bad-fg`                       | `#ffffff`                                         | `#2b0711`                                             | text on a `bad` fill (danger buttons)               |

`bad` is crimson rather than red so a failed bar never reads as a moving one. Each status colour
also has a `-wash` tint for badges and callouts.

Sizes around the 0.875rem body text (`text-sm`): `text-lead` (0.9375rem) for panel and section titles
and the dashboard summary, `text-small` (0.8125rem) for secondary lines, hints and metadata,
`text-micro` (0.6875rem) for key caps, chart axes and the phone navigation. Radii: `rounded-chip`
(4 px: tags, menu items, segmented options), `rounded-control` (6 px), `rounded-panel` (10 px). Use
these rather than arbitrary values.

Typeface: **Archivo** variable (width + weight axes), self-hosted. Width carries the role: 125 %
for the wordmark, 112 % for headings (`heading`), 100 % for text, 78 % for figures (`num`) and 64 %
for the hero readout (`readout`). Logs use the system monospace.

## Building blocks (`src/lib/components/ui`)

`Button`, `IconButton`, `DisclosureButton` (shows a row's files), `Toggle`, `Checkbox`, `Field`
(label + hint + error wiring), `TextInput`, `NumberInput`, `TimeInput`, `SearchInput`, `Select`,
`TextArea`, `LinesInput`, `TagInput`, `Segmented`, `Dialog` (native `<dialog>`), `ConfirmHost` (backs
`ui.confirm()`), `Menu` (Popover API), `Badge`, `StatusBadge`, `ProgressBar`, `Panel` (and
`PanelEmpty` for its empty list), `PageHeader` (page title and count), `Alert` (errors and warning
callouts), `EmptyState`, `Loading`, `Spinner`, `Toasts`.

Larger pieces: `dashboard/StatusHeader` (the sentence and `SpeedSpark`), `dashboard/ActivePackage`
(one package on the move), `dashboard/SectionHeading` (section title and count),
`dashboard/SegmentStrip` (segment map), `stats/ColumnChart` and `stats/BarList` (single-series
charts: muted ink, the current period in the signal colour, hover details, a table view on the page),
`layout/CommandPalette`, `add/GrabberDialog` (link grabber; grouping rules live in
`lib/utils/grouping.ts`). Shared class recipes live in
`ui/styles.ts`; status → colour mapping in `lib/utils/status.ts` (a `Tone` per package, file, link,
log level, toast and diagnostic, rendered through `toneText` / `toneFill` / `toneWash`); package
rules (moving, paused, progress) in `lib/utils/packages.ts`; all formatting in `lib/utils/format.ts`.
