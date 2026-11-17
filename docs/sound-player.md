Open a `.sound` or `.sound_looping` tag and the **Sound Player** sits at the top of its page, above the fields, and scrolls with them. It plays the tag's audio and extracts it to files.

## The header

- **Language**: a localized sound (Halo 2 dialogue, Halo 3 and later voice lines) starts the header with the language it plays and extracts. A language this sound has no audio for plays the default (English) instead; the menu greys it out and says why.
- **Permutation**: the menu picks one of the sound's permutations; the number in it is how many there are.
- **Shuffle**, **Skip Back**, **Play**, **Stop**, **Skip Forward** and **Loop**. Shuffle plays a random permutation, as the game picks; the Skip buttons step through them, and are greyed out when there is only one. Space plays or pauses; Enter stops, rewinds and clears the region.
- The **time**: where the playhead is, and how long the permutation is.
- **Speed**: type it, or step it with its arrows (10% at a time); double-click the word Speed for 100%. Pitch moves with the speed.
- **Volume**: click the speaker to mute, and again to put the volume back. Type into its box for more than 100%.
- **Zoom**, at the far right: **Fit** shows the whole sound, and the slider zooms between that and the closest view, with **−** and **+** at its ends. Fit and − are greyed out while the whole sound shows. Ctrl-scroll (Cmd on a Mac) over the waveform zooms too.

When the player is narrower than about 1,200 pixels, its header takes two rows: the language, permutation and volume on the first; the speed, then the buttons and time in the middle, then the zoom on the second.

## The waveform

A region you mark is read out under the waveform.

- **Click** the waveform to jump there.
- **Drag** across it to mark a region: Play plays just that part, and Loop repeats it. Esc, or a click outside it, clears it.
- **Drag** the playhead, or anywhere along the ruler above the waveform, to scrub.
- While zoomed, a strip under the waveform shows the whole sound, and dragging its box scrolls.

## The footer

A `.sound` tag's player has a footer. At the left, the **About** button opens this page. Beside it is what the sound is made of: its format, sample rate, channels and length, and the selected permutation's gain and skip fraction when it has them.

At the right are the **Extract** buttons:

- **Extract All** writes every permutation to a file. A tag in a kit's `tags` folder extracts beside it in the matching `data` folder (`data_<language>` for a language other than English); anywhere else, Baboon asks for a folder. A localized sound extracts the language picked in the header.
- **Extract All Languages** (Halo 2 localized sounds) extracts every language at once.
- **Extract** and the selected permutation's name extracts it alone, to a file you choose.
- **raw .ogg** (Halo CE) keeps the tag's original Ogg Vorbis audio instead of decoding it to WAV.

Hover a button to see where its files go and anything to know before reimporting them with Tool.

A `.sound_looping` tag's footer is much the same. Beside **About** are how many sounds it plays and the selected one's name, which opens that `.sound` tag. **Extract All** writes every sound it plays into their `data` folders, and **Extract** and the selected sound's name writes that one alone.

## The mini player

A sound keeps playing when you scroll its player out of sight or switch to another tag. While it does, a mini player sits in the bottom-right corner of the window with the header's **Shuffle**, **Skip Back**, **Play/Pause**, **Stop**, **Skip Forward** and **Loop**, and a bar showing how far through it is: hover it for a handle, and click or drag to move. The tag's folder sits above the buttons; hover it for the whole path. Click the tag's name at its top to go back to its tab, or drag the name to move the mini player somewhere else; it stays there until Baboon closes. It goes away when the sound stops or finishes, or when its own player is back in sight. Closing the sound's tab stops the sound.
