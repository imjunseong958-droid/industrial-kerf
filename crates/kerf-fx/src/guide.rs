//! What each carried effect does and when an editor reaches for it, as
//! agents read it in the catalog. One line each: the effect, then a few
//! moments or looks it serves.

pub(crate) const GUIDE: &[(&str, &str)] = &[
    // Blur & Sharpen
    (
        "blur.bilateral",
        "Smooths flat areas while keeping edges; skin smoothing and beauty passes.",
    ),
    (
        "blur.cross",
        "Blurs horizontally and vertically by separate amounts; streaky, stylized softness.",
    ),
    (
        "blur.radialzoom",
        "Streaks outward from a point like a fast zoom; impact hits and speed moments.",
    ),
    (
        "blur.radialfast",
        "Quick zoom blur from a center; zoom punch-ins and whoosh transitions.",
    ),
    (
        "blur.vector",
        "Blurs along the picture's own contours; painterly, flowing softness.",
    ),
    (
        "blur.channel",
        "Blurs red, green, blue, or alpha separately; color fringing and glitchy softness.",
    ),
    (
        "blur.directional",
        "Motion blur in one direction; fake fast motion and whip pans.",
    ),
    (
        "blur.fastbox",
        "Fast adjustable box blur; soft backgrounds, defocus, privacy blur.",
    ),
    (
        "blur.radial",
        "Spin or zoom blur around a center; whip spins, zoom hits, dizziness.",
    ),
    (
        "blur.sharpen",
        "Sharpens edges; crisp up soft footage, kept subtle.",
    ),
    (
        "blur.smart",
        "Blurs flat areas but keeps edges; clean, illustrated looks.",
    ),
    (
        "blur.unsharp",
        "Sharpens by contrast at edges; restores detail after scaling.",
    ),
    // Channel
    (
        "channel.arithmetic",
        "Per-channel math such as add or multiply; technical color tweaks.",
    ),
    (
        "channel.composite",
        "Blends the picture with itself by a mode; quick contrast and glow looks.",
    ),
    (
        "channel.invert",
        "Inverts colors or one channel; negative-film looks, flash frames, glitches.",
    ),
    (
        "channel.minimax",
        "Grows or shrinks bright or dark areas; thicker lines, choked mattes.",
    ),
    (
        "channel.removecolormatting",
        "Removes halos from premultiplied edges; cleanup after keying.",
    ),
    (
        "channel.shiftchannels",
        "Swaps or replaces color channels; surreal color and RGB-split looks.",
    ),
    (
        "channel.solidcomposite",
        "Composites the picture over a solid color; flat backgrounds and fills.",
    ),
    // Color Correction
    (
        "color.autocolor",
        "Automatic color balance; a quick fix for color casts.",
    ),
    (
        "color.autocontrast",
        "Automatic contrast; flat footage that needs punch.",
    ),
    (
        "color.autolevels",
        "Automatic levels per channel; quick normalization.",
    ),
    (
        "color.blackwhite",
        "Black and white with control per color; dramatic or retro looks.",
    ),
    (
        "color.brightnesscontrast",
        "Plain brightness and contrast; basic correction.",
    ),
    (
        "color.broadcast",
        "Keeps colors within broadcast-safe limits; TV delivery.",
    ),
    (
        "color.colorneutralizer",
        "Neutralizes shadows, midtones, and highlights to chosen colors; fixes casts.",
    ),
    (
        "color.coloroffset",
        "Rotates the color channels; trippy, stylized color shifts.",
    ),
    (
        "color.kernel",
        "A convolution kernel; custom sharpen and emboss filters.",
    ),
    (
        "color.toner",
        "Maps tones to two to five colors; duotones, stylized grades, brand colors.",
    ),
    (
        "color.changecolor",
        "Shifts the hue and saturation of one color; recolor an object.",
    ),
    (
        "color.changetocolor",
        "Replaces one color with another; recolor clothing or props.",
    ),
    (
        "color.channelmixer",
        "Mixes color channels into each other; creative grades, custom black and white.",
    ),
    (
        "color.colorbalance",
        "Shifts color in shadows, midtones, and highlights; warm or cool grading.",
    ),
    (
        "color.colorbalancehls",
        "Hue, lightness, and saturation over the whole picture; quick mood shifts.",
    ),
    (
        "color.equalize",
        "Spreads brightness evenly across the range; reveals detail, harsh look.",
    ),
    (
        "color.exposure",
        "Exposure in stops; fixes under- or overexposed shots.",
    ),
    (
        "color.gammapedestalgain",
        "Per-channel gamma, black level, and gain; technical grading.",
    ),
    (
        "color.huesaturation",
        "Hue, saturation, lightness, or colorize; desaturate, recolor, boost color.",
    ),
    (
        "color.leavecolor",
        "Keeps one color and desaturates the rest; color-pop highlights.",
    ),
    (
        "color.levels",
        "Input and output levels with gamma; contrast and exposure fixes.",
    ),
    (
        "color.levelsic",
        "Levels with controls per channel; precise grading.",
    ),
    (
        "color.ociocdl",
        "An ASC CDL (slope, offset, power, saturation); matching a colorist's grade.",
    ),
    (
        "color.photofilter",
        "A colored lens filter, warming or cooling; a quick mood tint.",
    ),
    (
        "color.selectivecolor",
        "Adjusts the color mix within color ranges; targeted color tweaks.",
    ),
    (
        "color.shadowhighlight",
        "Lifts shadows and recovers highlights; backlit or contrasty shots.",
    ),
    (
        "color.tint",
        "Maps black and white to two colors; duotones and monochrome tints.",
    ),
    (
        "color.tritone",
        "Maps shadows, midtones, and highlights to three colors; stylized grades.",
    ),
    (
        "color.vibrance",
        "Boosts muted colors more than saturated ones; natural pop.",
    ),
    (
        "color.videolimiter",
        "Limits luma and chroma to the legal range; delivery compliance.",
    ),
    // Distort
    (
        "distort.bezierwarp",
        "Warps the picture by curved edges; flag or paper bends, fitting to shapes.",
    ),
    (
        "distort.bulge",
        "Inflates or pinches a round area; comic face bulges, fisheye spots.",
    ),
    (
        "distort.bendit",
        "Bends the picture along a line; rubbery bends.",
    ),
    (
        "distort.bender",
        "Bends from a base toward a top; wobbly bending motion.",
    ),
    (
        "distort.flomotion",
        "Pulls the picture toward or away from two points; liquid warps.",
    ),
    (
        "distort.griddler",
        "Breaks the picture into a grid of turned tiles; mosaic tile looks.",
    ),
    (
        "distort.lens",
        "A refracting lens bubble; magnifying-glass effects.",
    ),
    (
        "distort.pageturn",
        "Curls the picture like a page; storybook page flips.",
    ),
    (
        "distort.powerpin",
        "Pins four corners in perspective; place a picture on a screen or sign.",
    ),
    (
        "distort.ripplepulse",
        "A ripple traveling outward in pulses; heartbeats and shock waves.",
    ),
    (
        "distort.slant",
        "Skews the picture; italic leans and a feeling of motion.",
    ),
    (
        "distort.pullsmear",
        "Smears pixels from one point toward another; stretchy pulls.",
    ),
    (
        "distort.smear",
        "Smears an area of the picture like liquid; warped highlights.",
    ),
    (
        "distort.split",
        "Splits the picture apart at a line; reveals and cracks.",
    ),
    (
        "distort.split2",
        "Splits the picture into halves that move apart; door-like reveals.",
    ),
    (
        "distort.tiler",
        "Repeats the picture as tiles; tiled walls and video grids.",
    ),
    (
        "distort.cornerpin",
        "Moves the four corners; fit a picture into a screen or frame.",
    ),
    (
        "distort.upscale",
        "Enlarges while keeping detail; low-resolution footage.",
    ),
    (
        "distort.magnify",
        "A magnifying circle or square; zoom in on a detail.",
    ),
    (
        "distort.mirror",
        "Mirrors the picture across a line; symmetric, surreal reflections.",
    ),
    (
        "distort.offset",
        "Shifts the picture with wraparound; seamless scrolls and glitch slides.",
    ),
    (
        "distort.opticscompensation",
        "Adds or removes lens barrel distortion; fisheye looks or wide-lens fixes.",
    ),
    (
        "distort.polar",
        "Converts between rectangular and polar coordinates; tiny planets, radial looks.",
    ),
    (
        "distort.ripple",
        "Concentric water ripples; dream sequences and water drops.",
    ),
    (
        "distort.spherize",
        "Wraps an area onto a sphere; bulging, fisheye accents.",
    ),
    (
        "distort.transform",
        "Position, scale, rotation, and skew with motion blur; precise moves.",
    ),
    (
        "distort.turbulentdisplace",
        "Organic noise displacement; heat haze, wobble, liquid.",
    ),
    (
        "distort.twirl",
        "Swirls the picture around a center; dizziness and vortex transitions.",
    ),
    (
        "distort.twirllegacy",
        "The older twirl; the same uses as Twirl.",
    ),
    (
        "distort.warp",
        "Arc, bulge, wave, fish, and other shape warps; text and logo warps.",
    ),
    (
        "distort.wavewarp",
        "Moving waves across the picture; flags, water, wobbling text.",
    ),
    // Generate
    (
        "generate.fourcolor",
        "A gradient from four colored corners; backgrounds and color washes.",
    ),
    (
        "generate.advancedlightning",
        "Lightning bolts; energy, magic, and electric hits.",
    ),
    (
        "generate.beam",
        "A laser beam between two points; lasers and sci-fi.",
    ),
    (
        "generate.gluegun",
        "Paints a glossy glue line; stylized strokes.",
    ),
    (
        "generate.lightburst",
        "Light rays bursting from a point; glowing reveals and titles.",
    ),
    (
        "generate.lightrays",
        "Rays of light from a point; god rays and dramatic light.",
    ),
    (
        "generate.lightsweep",
        "A streak of light sweeping across; shiny logo and text glints.",
    ),
    (
        "generate.threads",
        "A woven thread pattern; fabric textures.",
    ),
    (
        "generate.cellpattern",
        "Cellular patterns; organic textures and backgrounds.",
    ),
    (
        "generate.checkerboard",
        "A checkerboard; backgrounds and patterns.",
    ),
    (
        "generate.circle",
        "Draws a filled circle or ring; highlights and callouts.",
    ),
    (
        "generate.ellipse",
        "Draws an ellipse outline; callouts and rings.",
    ),
    (
        "generate.eyedropperfill",
        "Fills with a color sampled from the picture; matching flat fills.",
    ),
    (
        "generate.fractal",
        "Fractal patterns; psychedelic backgrounds.",
    ),
    (
        "generate.gradientramp",
        "A linear or radial gradient; skies, backgrounds, shading.",
    ),
    ("generate.grid", "Draws a grid; tech overlays and guides."),
    (
        "generate.lensflare",
        "A lens flare; bright light hits and cinematic sun flares.",
    ),
    (
        "generate.paintbucket",
        "Fills a region from a point; recolor flat areas.",
    ),
    (
        "generate.writeon",
        "Draws a stroke like handwriting as its point moves; underlines and doodles.",
    ),
    // Immersive Video
    (
        "vr.blur",
        "Blurs a 360° frame evenly on the sphere, seams and poles included; soft 360 backgrounds.",
    ),
    (
        "vr.chromaticaberrations",
        "Splits color channels around a point on a 360° frame; lens-like fringing in VR.",
    ),
    (
        "vr.colorgradients",
        "Lays soft color gradients over a 360° frame; tinting a whole VR sky or room.",
    ),
    (
        "vr.converter",
        "Converts between 360° equirectangular, cube maps, fisheye, and a flat view; reframing VR footage.",
    ),
    (
        "vr.denoise",
        "Reduces noise on a 360° frame without seams; cleaning VR footage.",
    ),
    (
        "vr.digitalglitch",
        "Glitch blocks and color shifts that wrap around a 360° frame; broken-signal looks in VR.",
    ),
    (
        "vr.fractalnoise",
        "Seamless animated noise on a 360° frame; fog, clouds, and textures that wrap in VR.",
    ),
    (
        "vr.glow",
        "Makes highlights glow evenly across a 360° frame; dreamy VR light.",
    ),
    (
        "vr.planetosphere",
        "Places a flat picture onto a 360° frame; titles and graphics inside VR.",
    ),
    (
        "vr.rotatesphere",
        "Turns a 360° frame around its axes; leveling the horizon or choosing the front view.",
    ),
    (
        "vr.sharpen",
        "Sharpens a 360° frame evenly without seams; crisper VR footage.",
    ),
    (
        "vr.spheretoplane",
        "Takes a flat view out of a 360° frame; framing a regular shot from VR footage.",
    ),
    // Keying
    (
        "key.advancedspill",
        "Removes green or blue spill; cleanup after keying.",
    ),
    (
        "key.simplewireremoval",
        "Removes a thin wire or rig between two points.",
    ),
    (
        "key.colordifference",
        "Keys blue or green screens by color difference; glass, smoke, fine detail.",
    ),
    ("key.colorrange", "Keys a range of colors; uneven screens."),
    (
        "key.extract",
        "Keys by brightness; removes black or white backgrounds.",
    ),
    (
        "key.innerouter",
        "Separates fine edges between inner and outer bounds; hair and fur.",
    ),
    (
        "key.keycleaner",
        "Restores detail at key edges; hair and motion blur.",
    ),
    (
        "key.screenmatte",
        "A full green or blue screen keyer with spill control; the main screen key.",
    ),
    (
        "key.linearcolor",
        "Keys a color by its distance from a key color; simple screen keys.",
    ),
    (
        "key.screen",
        "A screen keyer for green or blue backgrounds; quick screen keys.",
    ),
    (
        "key.unmult",
        "Makes black transparent; lay light leaks, fire, and sparks over footage.",
    ),
    // Matte
    (
        "matte.mattechoker",
        "Chokes or spreads a matte; tighter key edges.",
    ),
    (
        "matte.refinehard",
        "Refines hard matte edges; clean cutouts.",
    ),
    (
        "matte.simplechoker",
        "Shrinks or grows the matte edge; quick edge cleanup.",
    ),
    // Noise & Grain
    (
        "noise.addgrain",
        "Film grain; a filmic texture, or matching footage.",
    ),
    (
        "noise.curlnoise",
        "Curl noise; organic, swirling textures and warps.",
    ),
    (
        "noise.dustscratches",
        "Removes small specks and scratches; clean old scans.",
    ),
    (
        "noise.fractal",
        "Fractal noise; clouds, smoke, and textures.",
    ),
    (
        "noise.median",
        "A median filter; removes noise, painterly smoothing.",
    ),
    (
        "noise.medianlegacy",
        "The older median filter; the same uses as Median.",
    ),
    ("noise.noise", "Random pixel noise; static and grit."),
    (
        "noise.noisealpha",
        "Noise in the alpha channel; dissolving textures.",
    ),
    (
        "noise.noisehls",
        "Noise in hue, lightness, and saturation; analog grain and color noise.",
    ),
    (
        "noise.noisehlsauto",
        "Moving noise in hue, lightness, and saturation; TV static and analog flicker.",
    ),
    (
        "noise.removegrain",
        "Removes grain and noise; low-light footage.",
    ),
    (
        "noise.turbulent",
        "Turbulent noise; fire, smoke, and organic textures.",
    ),
    // Perspective
    (
        "perspective.bevelalpha",
        "A beveled edge on the picture's alpha; 3D-looking text and logos.",
    ),
    (
        "perspective.beveledges",
        "Beveled rectangular edges; button and tile looks.",
    ),
    (
        "perspective.cylinder",
        "Wraps the picture onto a cylinder; curved screens and turning labels.",
    ),
    (
        "perspective.sphere",
        "Wraps the picture onto a sphere; planets and balls.",
    ),
    (
        "perspective.spotlight",
        "A pool of spotlight; draws attention, stage lighting.",
    ),
    (
        "perspective.dropshadow",
        "A drop shadow; separates text and cutouts from the background.",
    ),
    (
        "perspective.radialshadow",
        "A shadow cast from a point light; realistic shadows.",
    ),
    // Simulation
    (
        "sim.ballaction",
        "Turns the picture into balls; dotted, ball-pixel looks.",
    ),
    (
        "sim.bubbles",
        "Rising bubbles; underwater and playful scenes.",
    ),
    (
        "sim.drizzle",
        "Raindrop ripples on a surface; rain on water.",
    ),
    (
        "sim.mrmercury",
        "Liquid metal blobs; melting and gooey effects.",
    ),
    (
        "sim.particlesystems2",
        "Particle bursts and fountains; sparks and fireworks.",
    ),
    (
        "sim.particleworld",
        "3D particle systems; magic dust, explosions, snow.",
    ),
    (
        "sim.pixelpolly",
        "Shatters the picture into polygons; breakup transitions.",
    ),
    ("sim.rainfall", "Falling rain streaks; rainy weather."),
    (
        "sim.scatterize",
        "Scatters the picture into particles; dissolve-away looks.",
    ),
    ("sim.snowfall", "Falling snow; winter scenes."),
    (
        "sim.starburst",
        "A star field bursting outward; warp speed and sparkle bursts.",
    ),
    // Stylize
    (
        "stylize.brushstrokes",
        "Painterly brush strokes; painting looks.",
    ),
    (
        "stylize.blockload",
        "Loads the picture in blocks; retro loading reveals.",
    ),
    (
        "stylize.burnfilm",
        "A hole burning through the film; film-burn transitions.",
    ),
    (
        "stylize.hextile",
        "Hexagon tiles; honeycomb and techy textures.",
    ),
    (
        "stylize.kaleida",
        "A kaleidoscope; trippy symmetric patterns.",
    ),
    (
        "stylize.repetile",
        "Repeats the picture past its edges; extended backgrounds, mirrored tiles.",
    ),
    (
        "stylize.smooththreshold",
        "A smooth black-and-white threshold; high-contrast graphic looks.",
    ),
    (
        "stylize.thresholdrgb",
        "A threshold per color channel; pop-art color posters.",
    ),
    (
        "stylize.vignette",
        "Darkens the edges; draws the eye to the center, cinematic mood.",
    ),
    (
        "stylize.cartoon",
        "Cartoon shading and outlines; comic looks.",
    ),
    (
        "stylize.coloremboss",
        "An emboss that keeps color; relief textures.",
    ),
    ("stylize.emboss", "A gray emboss; stamped relief looks."),
    (
        "stylize.findedges",
        "Outlines edges; sketch and line-art looks.",
    ),
    (
        "stylize.glow",
        "Makes bright areas glow; neon, dreamy highlights, glowing text.",
    ),
    (
        "stylize.mosaic",
        "Pixelates into blocks; censoring and retro 8-bit.",
    ),
    (
        "stylize.motiontile",
        "Tiles the picture with mirroring and offset; seamless pans, kaleidoscopes.",
    ),
    (
        "stylize.posterize",
        "Reduces the color levels; poster and pop-art looks.",
    ),
    (
        "stylize.roughenedges",
        "Roughens the picture's edges; torn paper, grunge text.",
    ),
    ("stylize.scatter", "Scatters pixels; grainy breakups."),
    (
        "stylize.strobe",
        "Flashes the picture on a rhythm; beat hits and strobe lights.",
    ),
    (
        "stylize.threshold",
        "A black-and-white threshold; graphic, photocopied looks.",
    ),
    // Text
    (
        "text.numbers",
        "Draws numbers, dates, or timecode on the picture; counters and data readouts.",
    ),
    (
        "text.timecode",
        "Burns the timecode or frame count into the picture; review copies and slates.",
    ),
    // Time
    (
        "time.echo",
        "Blends earlier or later frames over the current one; trails, ghosting, and motion echoes.",
    ),
    (
        "time.forcemotionblur",
        "Adds motion blur from the clip's own movement between frames; smoother fast action.",
    ),
    (
        "time.pixelmotionblur",
        "Motion blur from tracked pixel motion; natural blur on footage shot too sharp.",
    ),
    (
        "time.posterizetime",
        "Holds frames to play at a lower frame rate; choppy, stop-motion, or stylized movement.",
    ),
    (
        "time.widetime",
        "Blends several neighbouring frames; long-exposure smears and dreamy motion.",
    ),
    // Transitions
    (
        "transition.blockdissolve",
        "Dissolves in random blocks; digital, glitchy transitions.",
    ),
    (
        "transition.gridwipe",
        "A grid of tiles wipes away; techy reveals.",
    ),
    (
        "transition.iriswipe",
        "A polygon or star-shaped iris opens from a point; classic iris-out and spotlight reveals.",
    ),
    (
        "transition.jaws",
        "A jagged, teeth-like wipe; aggressive or comic transitions.",
    ),
    (
        "transition.lightwipe",
        "A bright light wipe; flashy, energetic transitions.",
    ),
    (
        "transition.linesweep",
        "A sweeping edge; clean graphic transitions.",
    ),
    (
        "transition.radialscalewipe",
        "A circular hole that grows; iris-like reveals.",
    ),
    (
        "transition.linearwipe",
        "A straight-edged wipe; classic wipes in any direction.",
    ),
    (
        "transition.radialwipe",
        "A clock-hand wipe; time passing, playful reveals.",
    ),
    (
        "transition.venetian",
        "Venetian blind stripes; retro TV transitions.",
    ),
    // Utility
    (
        "utility.overbrights",
        "Shows or clips overbright pixels; HDR checks.",
    ),
    (
        "utility.cineon",
        "Converts log Cineon footage; log to video.",
    ),
    (
        "utility.colorprofileconverter",
        "Converts between color profiles; technical conversion.",
    ),
    (
        "utility.growbounds",
        "Grows the layer bounds; lets effects reach past the picture.",
    ),
    (
        "utility.hdrcompander",
        "Compresses or expands the HDR range; technical.",
    ),
    (
        "utility.hdrcompression",
        "Compresses HDR highlights; tames clipped highlights.",
    ),
];
