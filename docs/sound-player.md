Open a `.sound` or `.sound_looping` tag and the **Sound Player** sits at the top of its page, above the fields, and scrolls with them. It plays the tag's audio and extracts it to files.

## The header

- **Language**: a localized sound (Halo 2 dialogue, Halo 3 and later voice lines) starts the header with the language it plays and extracts. A language this sound has no audio for plays the default (English) instead; the menu greys it out and says why.
- **Permutation**: the arrows step through the sound's permutations, the menu between them picks one, and the shuffle button plays a random one, as the game picks.
- **Play**, **Stop** and **Loop**. Space plays or pauses; Enter stops, rewinds and clears the region.

## The waveform

The row under the header shows the time, the zoom and the region, then the volume and speed.

- **Click** the waveform to jump there.
- **Drag** across it to mark a region: Play plays just that part, and Loop repeats it. Esc, or a click outside it, clears it.
- **Ctrl-scroll** (Cmd on a Mac) zooms; the strip above the waveform shows the whole sound, and dragging its box scrolls.
- **Volume** and **Speed** go past their sliders' ends if you type a value. Pitch moves with the speed.

## The footer

A `.sound` tag's player has a footer. At the left, the **About** button opens this page. Beside it is what the sound is made of: its format, sample rate, channels and length, and the selected permutation's gain and skip fraction when it has them.

At the right are the **Extract** buttons:

- **Extract all** writes every permutation to a file. A tag in a kit's `tags` folder extracts beside it in the matching `data` folder (`data_<language>` for a language other than English); anywhere else, Baboon asks for a folder. A localized sound extracts the language picked in the header.
- **All languages** (Halo 2 localized sounds) extracts every language at once.
- The button named after the selected permutation extracts it alone, to a file you choose.
- **raw .ogg** (Halo CE) keeps the tag's original Ogg Vorbis audio instead of decoding it to WAV.

Hover a button to see where its files go and anything to know before reimporting them with Tool.
