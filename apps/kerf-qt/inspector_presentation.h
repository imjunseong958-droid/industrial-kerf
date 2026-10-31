#pragma once

#include <QJsonObject>
#include <QString>

#include <optional>

namespace kerf {

// How the Inspector shows a number: the model keeps its own values (frame
// widths, fractions, ratios, dB) and the Inspector shows them in the units
// people read (px, %, °, dB, timecode). Display = model × scale.
struct NumberFormat {
    enum class Kind {
        Plain,    // value and unit
        Gain,     // dB with one decimal, "−∞" at the floor
        Pan,      // "L 40", "Centre", "R 25" (display −100…100)
        Speed,    // % with a minus sign for reverse
        Timecode, // frames shown as HH:MM:SS:FF (drop-frame with ;)
    };
    Kind kind = Kind::Plain;
    QString unit;        // after the value: px, %, °, dB, stops…
    bool tightUnit = false; // % and ° sit against the number
    QString prefix;      // X / Y before the value, in secondary
    double scale = 1.0;
    double min = -1e9;   // in display units
    double max = 1e9;
    int decimals = 0;
    bool showPlus = false; // "+0.40 stops" for values either side of zero
    double step = 1.0;     // ↑↓ and per 2px of drag
    double bigStep = 10.0; // with ⇧
    double smallStep = 0.1; // with ⌥
    double fps = 24.0;     // timecode
    bool dropFrame = false;
};

// The number as the field shows it, without its unit.
QString formatNumber(const NumberFormat &format, double display);
// The number with its unit, as a summary line shows it ("−8.0 dB", "112%").
QString formatWithUnit(const NumberFormat &format, double display);
// Frames as timecode on `fps` (drop-frame when asked and the rate allows).
QString frameTimecode(qint64 frames, double fps, bool dropFrame);
// Typed timecode: 00:00:07:00, 7:00 (seconds:frames), 12 (frames), 7s;
// with a leading + or − it is relative. Returns frames.
std::optional<qint64> parseTimecode(const QString &text, double fps, bool dropFrame, bool *relative);
// A typed number, in display units: plain, with its unit (50%, −3dB, 12px),
// relative (+10, and −10 for timecode), a factor (*2, 1.5x of a percentage),
// "−∞" for gain, L/R/Centre for pan. `current` is the value it replaces.
std::optional<double> parseNumber(const NumberFormat &format, const QString &text, double current);

// The context presentation needs: the sequence's frame and the clip's
// source picture.
struct PresentationContext {
    double sequenceWidth = 1920.0;
    double sequenceHeight = 1080.0;
    double sourceWidth = 0.0; // 0: the clip draws the sequence's frame
    double sourceHeight = 0.0;
};

// The format of the catalog property or effect parameter `def` (its name,
// range, unit and default), from the Inspector's presentation table:
// x / y in px of the sequence, scale and opacity in %, rotation in °, crops
// in px of the source, anchors in px of the clip, gain in dB, pan L…R, a
// title's size in px of the sequence height, mask feather and expansion in
// px and opacity in %; anything else by the catalog unit, or by its range.
NumberFormat presentationFor(const QJsonObject &def, const PresentationContext &context);
// A row's label: the presentation table's, else the catalog's own doc
// (after its "Group: " prefix) or its name, capitalised.
QString presentationLabel(const QJsonObject &def);
// "Edge Threshold" → "Edge threshold" (acronyms and the first word keep
// their capitals), with "color" written "colour" as in the rest of the UI.
QString sentenceCase(const QString &text);
// "color" → "colour" (and "Color" → "Colour") in a string shown to people.
QString britishSpelling(const QString &text);
// An effect's name as the Inspector shows it: Title Case without a final
// full stop, "Colour" spelt as elsewhere ("Primary Colour Correction").
QString presentationEffectName(const QString &name);
// A parameter's label without a first word its effect's name already says
// ("Edge threshold" under "Edge Glow" → "Threshold"); unchanged otherwise.
QString withoutSharedWord(const QString &label, const QString &effectName);
// The "Group" of a "Group: Name" doc, else empty.
QString presentationGroup(const QJsonObject &def);

// Gain fader travel (0…1) for dB, 0 dB at 75% with −96 at the floor, and back.
double gainToTravel(double db);
double travelToGain(double travel);

} // namespace kerf
