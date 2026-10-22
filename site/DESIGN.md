# VRCLT — editorial product direction

The page introduces a Windows voice translation tool to VRChat and Discord
users. The primary action is to download the latest public release. v1.0 is
released; the gallery uses illustrative app screens with sample conversations.

## References inspected

- [Electronic Materials Office on Awwwards](https://www.awwwards.com/sites/electronic-materials-office)
  and its live product site: product-scale imagery, disciplined captions,
  technical details explained visually.
- [Jitter on Awwwards](https://www.awwwards.com/sites/jitter) and its live site:
  decisive headline scale, product examples as the main visual content.
- [CoLabs on CSS Design Awards](https://www.cssdesignawards.com/sites/colabs/47352/)
  and its live site: asymmetry and a clear dominant image with a smaller
  supporting rail.

These informed hierarchy and proportion. Their assets, copy and layouts were
not copied. Live reference sites may differ from their award submission.

## Decisions

- Cool paper `#f1f3f2`, graphite `#17232c`, steel `#53616c`, pale blue
  `#d1dee7`, dark gallery `#18252f`, brand blue `#8cc8f2`.
- Noto Sans CJK in two locally served weights; Georgia display type in English;
  system monospace only for the setup sequence.
- A large, asymmetric headline spread leads into the real desktop interface.
  Three capabilities occupy a narrow annotation column, not a card grid.
- The dark gallery starts with VR subtitles; tabs expose the other actual
  interface renders. An audio-routing diagram explains the setup.
- No stock imagery, fake usage metrics, decorative waveforms or parallax.
  The screenshots are explicitly labeled as app renders with sample data.
- Describe features directly instead of forcing rhyme or paired slogan fragments.
  Preserve localized wording supplied by the user or their native-language reviewer.
- Let the headline, product image and spacing establish the hierarchy. Omit
  decorative eyebrows, screenshot labels and repeated release announcements.
  Tabs identify the gallery views; captions only add information the image
  cannot show, such as engine-dependent speaker labels and sample provenance.
- Present setup as three columns in reading order, followed by a single audio
  path. On small screens both become a vertical sequence. Keep API costs,
  installation requirements and privacy information near their relevant actions.
- Engine names appear once in a linked list. The final download prompt needs
  only its heading and action, without another explanatory paragraph.
- Keep the audio path in four equal columns with centered nodes and arrows
  halfway between them. Mobile keeps the same sequence from top to bottom.
- Motion is brief and functional: a one-time section entrance, gallery crossfade
  and small hover/disclosure responses. Use opacity and transforms, honor reduced
  motion, and leave all content visible when JavaScript is unavailable.

Review at 320, 390, 768 and 1440 px in all five languages. Verify keyboard tabs,
language navigation, FAQ disclosures, image loading and release destinations.
