//! What each carried effect does and when an editor reaches for it, as
//! agents read it in the catalog. One line each: the effect, then a few
//! moments or looks it serves.

pub(crate) const GUIDE: &[(&str, &str)] = &[
    // Blur & Sharpen
    (
        "blur.camerashakedeblur",
        "Swaps frames blurred by camera shake for sharper neighbouring frames; handheld shots with smeared frames.",
    ),
    (
        "blur.cameralens",
        "Lens-shaped defocus with bokeh, shaped by a depth map from another track (blurMap/blurMapLayer=V2); realistic focus falloff and rack focus.",
    ),
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
        "blur.fastbox",
        "Fast adjustable box blur; soft backgrounds, defocus, privacy blur.",
    ),
    (
        "blur.radial",
        "Spin or zoom blur around a center; whip spins, zoom hits, dizziness.",
    ),
    (
        "blur.smart",
        "Blurs flat areas but keeps edges; clean, illustrated looks.",
    ),
    // Channel
    (
        "channel.blend",
        "Blends with another track's picture by a chosen mode (blendWithLayer=V2); quick double exposures and color mixes.",
    ),
    (
        "channel.calculations",
        "Combines channels of the clip and another track (secondLayer=V2) by a blending mode; channel-based composites and mattes.",
    ),
    (
        "channel.combiner",
        "Converts between color models or splits channels, optionally from another track (sourceLayer=V2); technical channel work.",
    ),
    (
        "channel.compoundarithmetic",
        "Adds, subtracts, or multiplies the clip with another track (secondSource=V2); difference checks and light composites.",
    ),
    (
        "channel.setchannels",
        "Builds each channel from a channel of other tracks (sourceLayer1=V2 and so on); channel swaps and custom mattes.",
    ),
    (
        "channel.setmatte",
        "Takes the clip's transparency from another track (takeMatteFromLayer=V2); track mattes, text and shape masks.",
    ),
    (
        "channel.arithmetic",
        "Per-channel math such as add or multiply; technical color tweaks.",
    ),
    (
        "channel.composite",
        "Blends the picture with itself by a mode; quick contrast and glow looks.",
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
        "color.colorstabilizer",
        "Holds brightness, levels, or curves steady against a reference frame (referenceFrame in seconds); flicker from changing light or auto exposure.",
    ),
    (
        "color.colorlink",
        "Tints the clip with the average color of another track (sourceLayer=V2); matching a shot's color to a background plate.",
    ),
    (
        "color.colorama",
        "Cycles colors through a palette by luminance, optionally phased or masked by other tracks (inputPhase/addPhase=V2); psychedelic and thermal looks.",
    ),
    (
        "color.curves",
        "Bends tone with curves for RGB and each channel, each written as points x,y from 0 to 1 (rgb=\"0,0 0.5,0.6 1,1\" lifts the mids); contrast, fades, and color casts.",
    ),
    (
        "color.arbitrarymap",
        "Remaps every level through a table of 256 output levels (map=\"255,254 … 0\" inverts); posterized, solarized, and graphic tone maps.",
    ),
    (
        "color.ociocolorspace",
        "Converts between two color spaces named in an OpenColorIO config; log footage to Rec.709, ACES workflows.",
    ),
    (
        "color.ociodisplay",
        "Shows the picture through an OpenColorIO display and view; previewing log or ACES footage as it will be seen.",
    ),
    (
        "color.ociofile",
        "Applies a color transform file (.cube, .3dl, .cdl, and other LUT formats) through OpenColorIO; camera LUTs and CDL grades.",
    ),
    (
        "color.ociolook",
        "Applies looks named in an OpenColorIO config; show looks and creative grades from a pipeline.",
    ),
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
        "color.tritone",
        "Maps shadows, midtones, and highlights to three colors; stylized grades.",
    ),
    (
        "color.vibrance",
        "Boosts muted colors more than saturated ones; natural pop.",
    ),
    // Distort
    (
        "distort.rollingshutterrepair",
        "Straightens the skew and wobble a rolling shutter leaves, from the motion between frames; fast pans on phones and DSLRs.",
    ),
    (
        "distort.blobbylize",
        "Melts the picture into glossy blobs shaped by another track (blobLayer=V2); liquid and gooey looks.",
    ),
    (
        "distort.displacementmap",
        "Pushes pixels by another track's brightness (displacementMapLayer=V2); heat haze, water ripples, glass, and warped reveals.",
    ),
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
        "distort.reshape",
        "Morphs the picture from one mask's outline to another's (sourceMask=k1 destinationMask=k2), inside a boundary mask; reshaping objects and shape morphs.",
    ),
    (
        "distort.smear",
        "Drags the area inside one mask by an offset, held inside a boundary mask (sourceMask=k1 boundaryMask=k2); liquid pulls and warped highlights.",
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
        "distort.upscale",
        "Enlarges while keeping detail; low-resolution footage.",
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
        "distort.twirllegacy",
        "The older twirl; the same uses as Twirl.",
    ),
    (
        "distort.warp",
        "Arc, bulge, wave, fish, and other shape warps; text and logo warps.",
    ),
    // Generate
    (
        "generate.audiospectrum",
        "Draws a sound track's frequencies as bars or lines (audioLayer=A1), along a mask if one is named; music visualizers and podcast clips.",
    ),
    (
        "generate.audiowaveform",
        "Draws a sound track's waveform as a line (audioLayer=A1), along a mask if one is named; voice clips and music visualizers.",
    ),
    (
        "generate.fill",
        "Fills one of the clip's masks with a color (fillMask=k1); shapes, highlights, and flat graphic panels.",
    ),
    (
        "generate.radiowaves",
        "Rings ripple out from a point or a mask's outline (waveMask/mask=k1); sonar, signal, and pulse graphics.",
    ),
    (
        "generate.scribble",
        "Scribbles inside or along a mask (mask=k1), drawing on over time; hand-drawn fills and highlights.",
    ),
    (
        "generate.stroke",
        "Paints along a mask's outline, which can draw on from start to end (path=k1); outlines, circled details, and write-on lines.",
    ),
    (
        "generate.vegas",
        "Light runs along a mask's outline (path=k1) or another track's contours; neon chases and marquee lights.",
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
        "generate.eyedropperfill",
        "Fills with a color sampled from the picture; matching flat fills.",
    ),
    (
        "generate.fractal",
        "Fractal patterns; psychedelic backgrounds.",
    ),
    // Immersive Video
    (
        "vr.converter",
        "Converts between 360° equirectangular, cube maps, fisheye, and a flat view; reframing VR footage.",
    ),
    (
        "vr.spheretoplane",
        "Takes a flat view out of a 360° frame; framing a regular shot from VR footage.",
    ),
    // Keying
    (
        "key.differencematte",
        "Keys out what matches a clean plate on another track (differenceLayer=V2); removing a static background without a green screen.",
    ),
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
        "matte.refinesoft",
        "Refines a key's soft edges, such as hair and motion blur, and steadies edges that chatter between frames; after a key.",
    ),
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
        "noise.matchgrain",
        "Copies the grain of another track onto the clip (noiseSourceLayer=V2); making inserts and graphics sit in grainy footage.",
    ),
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
        "noise.medianlegacy",
        "The older median filter; the same uses as Median.",
    ),
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
        "perspective.3dglasses",
        "Combines a left and a right view from two tracks (leftView=V2 rightView=V3) into anaglyph or side-by-side 3D; stereo 3D output.",
    ),
    (
        "perspective.environment",
        "Reflects another track as an environment around the picture (environment=V2); shiny, chrome, and reflective surfaces.",
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
        "perspective.radialshadow",
        "A shadow cast from a point light; realistic shadows.",
    ),
    // Simulation
    (
        "sim.particleplayground",
        "Fires particles from a cannon, grid, or exploding picture, walled in by a mask (wall/wallBoundary=k1); confetti, sparks, and dissolving pictures.",
    ),
    (
        "sim.hair",
        "Grows hair or fur over the picture, its fall shaped by another track (hairfallMap/mapLayer=V2); fuzzy, furry, and grassy textures.",
    ),
    (
        "sim.carddance",
        "Splits the picture into cards that move by gradient tracks (gradientLayer1=V2) with another track on their backs (backLayer=V3); card-flip reveals and mosaics.",
    ),
    (
        "sim.caustics",
        "Light ripples like sunlight through water, using tracks as the bottom, water surface, and sky (water/waterSurface=V2); pools and underwater light.",
    ),
    (
        "sim.foam",
        "Bubbles that float and flow, textured or steered by other tracks (flowMap/flowMap=V2); foam, bubbles, and particle swarms.",
    ),
    (
        "sim.shatter",
        "Breaks the picture into flying pieces, optionally by a gradient track (gradient/gradientLayer=V2); explosions and glass-shatter transitions.",
    ),
    (
        "sim.waveworld",
        "Simulates water waves, with another track as the ground they run over (ground/ground=V2); wave maps for displacement and caustics.",
    ),
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
        "stylize.glass",
        "Distorts the picture as if seen through glass bumps from another track (bumpMap=V2); frosted and textured glass.",
    ),
    (
        "stylize.mrsmoothie",
        "Smears color along the flow of another track (flowLayer=V2); psychedelic, painterly color flow.",
    ),
    (
        "stylize.plastic",
        "Gives the picture a shiny raised surface from another track's bumps (bumpLayer=V2); embossed plastic and molded looks.",
    ),
    (
        "stylize.texturize",
        "Embosses another track's texture into the picture (textureLayer=V2); canvas, paper, and surface textures.",
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
        "stylize.cartoon",
        "Cartoon shading and outlines; comic looks.",
    ),
    (
        "stylize.glow",
        "Makes bright areas glow; neon, dreamy highlights, glowing text.",
    ),
    (
        "stylize.motiontile",
        "Tiles the picture with mirroring and offset; seamless pans, kaleidoscopes.",
    ),
    ("stylize.scatter", "Scatters pixels; grainy breakups."),
    (
        "stylize.threshold",
        "A black-and-white threshold; graphic, photocopied looks.",
    ),
    // Text
    (
        "text.numbers",
        "Draws numbers, dates, or timecode on the picture; counters and data readouts.",
    ),
    // Time
    (
        "time.timedifference",
        "Shows the difference between the clip and another track over time (targetLayer=V2); spotting motion and changes between takes.",
    ),
    (
        "time.timedisplacement",
        "Shows each pixel from a different moment by another track's brightness (displacementMapLayer=V2); time slices and rolling-shutter-style warps.",
    ),
    (
        "time.timewarp",
        "Retimes the clip with frame blending or motion-estimated frames, optionally guided by other tracks (warpLayer=V2); smooth slow motion and speed ramps.",
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
        "time.widetime",
        "Blends several neighbouring frames; long-exposure smears and dreamy motion.",
    ),
    // Transitions
    (
        "transition.cardwipe",
        "Cards flip over to reveal the next shot, optionally timed by a gradient track (gradientLayer=V2); techy grid flips.",
    ),
    (
        "transition.glasswipe",
        "Wipes through rippled glass shaped by a gradient track (gradientLayer=V2); liquid, refractive transitions.",
    ),
    (
        "transition.gradientwipe",
        "Wipes in the order of a gradient track's brightness, dark first (gradientLayer=V2); custom-shaped wipes from any picture.",
    ),
    (
        "transition.imagewipe",
        "Wipes by the brightness of another track (layer=V2); wipes shaped by a texture or logo.",
    ),
    (
        "transition.twister",
        "Twists the picture away like a ribbon, with another track on its back (backside=V2); playful spinning transitions.",
    ),
    (
        "transition.warpomatic",
        "Warps the picture away by the brightness of another track (reactorLayer=V2); melting, organic transitions.",
    ),
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
        "utility.applylut",
        "Applies a .cube LUT file; camera log to Rec.709, film emulation, and ready-made looks.",
    ),
    (
        "utility.overbrights",
        "Shows or clips overbright pixels; HDR checks.",
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
