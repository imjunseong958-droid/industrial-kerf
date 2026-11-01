#include "inspector_presentation.h"

#include <QRegularExpression>
#include <QStringList>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

const QChar kMinus(0x2212);

QString number(double value, int decimals, bool trim) {
    QString text = QString::number(std::abs(value), 'f', std::max(0, decimals));
    if (trim && text.contains(QLatin1Char('.'))) {
        while (text.endsWith(QLatin1Char('0'))) text.chop(1);
        if (text.endsWith(QLatin1Char('.'))) text.chop(1);
    }
    return text;
}

bool isZero(const QString &digits) {
    for (const QChar c : digits)
        if (c.isDigit() && c != QLatin1Char('0')) return false;
    return true;
}

// The nominal rate of timecode on `fps` (30 for 29.97) and the frame
// numbers drop-frame skips each minute, or 0 when it does not apply.
int nominalRate(double fps) {
    return std::max(1, static_cast<int>(std::lround(fps)));
}

int droppedPerMinute(double fps, bool dropFrame) {
    if (!dropFrame) return 0;
    const int nominal = nominalRate(fps);
    if (std::abs(fps - nominal) < 1e-3 || (nominal != 30 && nominal != 60)) return 0;
    return nominal / 15;
}

} // namespace

QString frameTimecode(qint64 frames, double fps, bool dropFrame) {
    const int nominal = nominalRate(fps);
    const int drop = droppedPerMinute(fps, dropFrame);
    const bool negative = frames < 0;
    qint64 count = std::llabs(frames);
    if (drop > 0) {
        const qint64 perTenMinutes = std::llround(fps * 600.0);
        const qint64 perMinute = qint64(nominal) * 60 - drop;
        const qint64 tens = count / perTenMinutes;
        const qint64 rest = count % perTenMinutes;
        count += qint64(drop) * 9 * tens;
        if (rest > drop) count += qint64(drop) * ((rest - drop) / perMinute);
    }
    const qint64 frame = count % nominal;
    qint64 whole = count / nominal;
    const qint64 seconds = whole % 60;
    whole /= 60;
    const qint64 minutes = whole % 60;
    const qint64 hours = whole / 60;
    const auto two = [](qint64 value) { return QStringLiteral("%1").arg(value, 2, 10, QLatin1Char('0')); };
    return (negative ? QString(kMinus) : QString()) + two(hours) + QLatin1Char(':') + two(minutes) + QLatin1Char(':') +
           two(seconds) + (drop > 0 ? QLatin1Char(';') : QLatin1Char(':')) + two(frame);
}

std::optional<qint64> parseTimecode(const QString &input, double fps, bool dropFrame, bool *relative) {
    QString text = input.trimmed();
    text.replace(kMinus, QLatin1Char('-'));
    if (relative) *relative = false;
    int sign = 1;
    if (text.startsWith(QLatin1Char('+')) || text.startsWith(QLatin1Char('-'))) {
        sign = text.startsWith(QLatin1Char('-')) ? -1 : 1;
        if (relative) *relative = true;
        text = text.mid(1).trimmed();
    }
    if (text.isEmpty()) return std::nullopt;
    if (text.endsWith(QLatin1Char('s'), Qt::CaseInsensitive)) {
        bool ok = false;
        const double seconds = text.chopped(1).trimmed().toDouble(&ok);
        if (!ok) return std::nullopt;
        return sign * std::llround(seconds * fps);
    }
    static const QRegularExpression separators(QStringLiteral("[:;.,]"));
    const QStringList parts = text.split(separators);
    if (parts.size() > 4) return std::nullopt;
    QList<qint64> values;
    for (const QString &part : parts) {
        bool ok = false;
        const qint64 value = part.isEmpty() ? 0 : part.toLongLong(&ok);
        if (!part.isEmpty() && !ok) return std::nullopt;
        values.prepend(value);
    }
    // values: frames, seconds, minutes, hours
    while (values.size() < 4) values.append(0);
    const int nominal = nominalRate(fps);
    const qint64 minutes = values.at(3) * 60 + values.at(2);
    qint64 frames = ((minutes * 60) + values.at(1)) * nominal + values.at(0);
    if (const int drop = droppedPerMinute(fps, dropFrame); drop > 0 && parts.size() > 2)
        frames -= qint64(drop) * (minutes - minutes / 10);
    return sign * frames;
}

QString formatNumber(const NumberFormat &format, double display) {
    switch (format.kind) {
    case NumberFormat::Kind::Timecode:
        return frameTimecode(std::llround(display), format.fps, format.dropFrame);
    case NumberFormat::Kind::Gain: {
        if (display <= format.min + 1e-6) return QString(kMinus) + QChar(0x221e);
        const QString digits = number(display, 1, false);
        if (isZero(digits)) return QStringLiteral("0.0");
        return (display < 0 ? QString(kMinus) : QStringLiteral("+")) + digits;
    }
    case NumberFormat::Kind::Pan: {
        const long rounded = std::lround(display);
        if (rounded == 0) return QStringLiteral("Centre");
        return QStringLiteral("%1 %2").arg(rounded < 0 ? QStringLiteral("L") : QStringLiteral("R")).arg(std::labs(rounded));
    }
    case NumberFormat::Kind::Speed: {
        const QString digits = number(display, 1, true);
        return (display < 0 && !isZero(digits) ? QString(kMinus) : QString()) + digits;
    }
    case NumberFormat::Kind::Plain:
        break;
    }
    const QString digits = number(display, format.decimals, format.decimals > 0 && format.step >= 1.0);
    if (isZero(digits)) return digits;
    if (display < 0) return QString(kMinus) + digits;
    return format.showPlus ? QStringLiteral("+") + digits : digits;
}

QString formatWithUnit(const NumberFormat &format, double display) {
    const QString text = formatNumber(format, display);
    if (format.kind == NumberFormat::Kind::Pan || format.kind == NumberFormat::Kind::Timecode || format.unit.isEmpty()) return text;
    if (format.kind == NumberFormat::Kind::Gain && display <= format.min + 1e-6) return text;
    return text + (format.tightUnit ? QString() : QStringLiteral(" ")) + format.unit;
}

std::optional<double> parseNumber(const NumberFormat &format, const QString &input, double current) {
    QString text = input.trimmed().toLower();
    text.replace(kMinus, QLatin1Char('-'));
    text.replace(QChar(0x00d7), QLatin1Char('*'));
    if (text.isEmpty()) return std::nullopt;
    if (format.kind == NumberFormat::Kind::Timecode) {
        bool relative = false;
        const std::optional<qint64> frames = parseTimecode(text, format.fps, format.dropFrame, &relative);
        if (!frames) return std::nullopt;
        return relative ? current + double(*frames) : double(*frames);
    }
    if (format.kind == NumberFormat::Kind::Gain &&
        (text.contains(QChar(0x221e)) || text.contains(QStringLiteral("inf"))))
        return format.min;
    if (format.kind == NumberFormat::Kind::Pan) {
        if (text == QStringLiteral("c") || text.startsWith(QStringLiteral("cent"))) return 0.0;
        const bool left = text.startsWith(QLatin1Char('l'));
        if (left || text.startsWith(QLatin1Char('r'))) {
            bool ok = false;
            const double amount = text.mid(1).trimmed().toDouble(&ok);
            if (!ok) return std::nullopt;
            return left ? -std::abs(amount) : std::abs(amount);
        }
    }
    enum class Mode { Absolute, Relative, Factor } mode = Mode::Absolute;
    if (text.startsWith(QLatin1Char('*'))) {
        mode = Mode::Factor;
        text = text.mid(1).trimmed();
    } else if (text.startsWith(QLatin1Char('+'))) {
        mode = Mode::Relative;
        text = text.mid(1).trimmed();
    }
    // A trailing "x" is a factor of 100% where the field is a percentage.
    bool times = false;
    static const QRegularExpression trailing(QStringLiteral("^([-+]?[0-9]*\\.?[0-9]+(?:e[-+]?[0-9]+)?)\\s*([a-z%°]*)$"));
    const QRegularExpressionMatch match = trailing.match(text);
    if (!match.hasMatch()) return std::nullopt;
    const double amount = match.captured(1).toDouble();
    const QString unit = match.captured(2);
    if (unit == QStringLiteral("x")) times = true;
    double value = amount;
    const bool percentField = format.unit == QStringLiteral("%") || format.kind == NumberFormat::Kind::Speed;
    if (times && percentField && mode != Mode::Factor) value = amount * 100.0;
    switch (mode) {
    case Mode::Absolute:
        return value;
    case Mode::Relative:
        return current + value;
    case Mode::Factor:
        return current * amount;
    }
    return value;
}

namespace {

void decimalsForRange(NumberFormat &format) {
    const double span = format.max - format.min;
    if (span > 1e8) format.decimals = format.scale >= 100.0 ? 0 : 2;
    else format.decimals = span > 100.0 ? 0 : span > 10.0 ? 1 : 2;
}

} // namespace

NumberFormat presentationFor(const QJsonObject &def, const PresentationContext &context) {
    NumberFormat format;
    const QString name = def.value(QStringLiteral("name")).toString();
    const QString unit = def.value(QStringLiteral("unit")).toString();
    const double defaultValue = def.value(QStringLiteral("default")).toDouble();
    const bool bounded = def.contains(QStringLiteral("min")) && def.contains(QStringLiteral("max"));
    const double modelMin = def.value(QStringLiteral("min")).toDouble(-1e9);
    const double modelMax = def.value(QStringLiteral("max")).toDouble(1e9);
    const double pictureWidth = context.sourceWidth > 0 ? context.sourceWidth : context.sequenceWidth;
    const double pictureHeight = context.sourceHeight > 0 ? context.sourceHeight : context.sequenceHeight;
    // Rows of the table keep its steps and decimals; the rest take theirs
    // from their range.
    bool tabled = true;
    const auto pixels = [&format](double scale, const QString &prefix = QString()) {
        format.unit = QStringLiteral("px");
        format.scale = scale;
        format.prefix = prefix;
        format.decimals = 1;
        format.step = 1.0;
        format.bigStep = 10.0;
        format.smallStep = 0.1;
    };
    const auto percent = [&format](double scale) {
        format.unit = QStringLiteral("%");
        format.tightUnit = true;
        format.scale = scale;
        format.decimals = 1;
        format.step = 1.0;
        format.bigStep = 10.0;
        format.smallStep = 0.1;
    };
    // The presentation table: the clip's own properties, a title's size and
    // the masks' parameters.
    if (name == QStringLiteral("x") && unit == QStringLiteral("frame widths")) {
        pixels(context.sequenceWidth, QStringLiteral("X"));
    } else if (name == QStringLiteral("y") && unit == QStringLiteral("frame heights")) {
        pixels(context.sequenceHeight, QStringLiteral("Y"));
    } else if ((name == QStringLiteral("scale") || name == QStringLiteral("scale_x") || name == QStringLiteral("scale_y")) &&
               unit == QStringLiteral("×")) {
        percent(100.0);
    } else if (name == QStringLiteral("rotation") && unit == QStringLiteral("degrees")) {
        format.unit = QStringLiteral("°");
        format.tightUnit = true;
        format.decimals = 1;
        format.step = 1.0;
        format.bigStep = 15.0;
        format.smallStep = 0.1;
    } else if (name.startsWith(QStringLiteral("crop_")) && unit == QStringLiteral("fraction")) {
        const bool across = name == QStringLiteral("crop_l") || name == QStringLiteral("crop_r");
        pixels(across ? pictureWidth : pictureHeight);
        format.decimals = 0;
        format.smallStep = 1.0;
    } else if (name == QStringLiteral("anchor_x") || name == QStringLiteral("anchor_y")) {
        pixels(name == QStringLiteral("anchor_x") ? pictureWidth : pictureHeight,
               name == QStringLiteral("anchor_x") ? QStringLiteral("X") : QStringLiteral("Y"));
        format.decimals = 0;
        format.smallStep = 1.0;
    } else if (name == QStringLiteral("gain") && unit == QStringLiteral("dB")) {
        format.kind = NumberFormat::Kind::Gain;
        format.unit = QStringLiteral("dB");
        format.decimals = 1;
        format.step = 0.5;
        format.bigStep = 3.0;
        format.smallStep = 0.1;
    } else if (name == QStringLiteral("pan") && bounded && modelMin == -1.0 && modelMax == 1.0) {
        format.kind = NumberFormat::Kind::Pan;
        format.scale = 100.0;
        format.step = 1.0;
        format.bigStep = 10.0;
        format.smallStep = 1.0;
    } else if (unit == QStringLiteral("frame heights")) {
        pixels(context.sequenceHeight);
        format.decimals = 0;
        format.smallStep = 1.0;
    } else {
        tabled = false;
        if (unit == QStringLiteral("frame widths") || unit == QStringLiteral("picture widths")) {
            format.unit = QStringLiteral("px");
            format.scale = context.sequenceWidth;
        } else if (unit == QStringLiteral("picture heights")) {
            format.unit = QStringLiteral("px");
            format.scale = context.sequenceHeight;
        } else if (unit == QStringLiteral("pixels") || unit == QStringLiteral("px")) {
            format.unit = QStringLiteral("px");
        } else if (unit == QStringLiteral("degrees")) {
            format.unit = QStringLiteral("°");
            format.tightUnit = true;
        } else if (unit == QStringLiteral("%")) {
            format.unit = QStringLiteral("%");
            format.tightUnit = true;
        } else if (unit == QStringLiteral("fraction") || (unit.isEmpty() && bounded && modelMin >= 0.0 && defaultValue == 1.0)) {
            format.unit = QStringLiteral("%");
            format.tightUnit = true;
            format.scale = 100.0;
        } else if (unit.isEmpty() && bounded && modelMin == -1.0 && modelMax == 1.0) {
            format.scale = 100.0;
            format.showPlus = true;
        } else if (unit.isEmpty() && (name.contains(QStringLiteral("radius"), Qt::CaseInsensitive) ||
                                      name.contains(QStringLiteral("width"), Qt::CaseInsensitive))) {
            // A radius or a width with no unit in the catalog is a distance
            // in pixels of the picture.
            format.unit = QStringLiteral("px");
        } else {
            // Exposure's stops read as EV, short enough for the field.
            format.unit = unit == QStringLiteral("stops") ? QStringLiteral("EV") : unit;
            format.showPlus = unit == QStringLiteral("stops") || unit == QStringLiteral("K") ||
                              (bounded && modelMin < 0.0 && modelMax > 0.0);
        }
    }
    format.min = bounded || def.contains(QStringLiteral("min")) ? modelMin * format.scale : -1e9;
    format.max = bounded || def.contains(QStringLiteral("max")) ? modelMax * format.scale : 1e9;
    if (format.kind == NumberFormat::Kind::Gain) {
        format.min = std::max(format.min, -96.0);
        format.max = std::min(format.max, 24.0);
    }
    // Effect parameters with no table row: decimals from the range; a step
    // of 1% of it, ×10 with ⇧ and ÷10 with ⌥.
    if (!tabled) {
        decimalsForRange(format);
        const double span = format.max - format.min;
        if (span < 1e8) {
            format.step = span / 100.0;
            const double quantum = std::pow(10.0, -format.decimals);
            format.step = std::max(quantum, std::round(format.step / quantum) * quantum);
        } else {
            format.step = format.decimals == 0 ? 1.0 : std::pow(10.0, -format.decimals + 1);
        }
        format.bigStep = format.step * 10.0;
        format.smallStep = std::max(std::pow(10.0, -format.decimals), format.step / 10.0);
    }
    return format;
}

QString presentationGroup(const QJsonObject &def) {
    const QString doc = def.value(QStringLiteral("doc")).toString();
    const int colon = doc.indexOf(QStringLiteral(": "));
    return colon > 0 && colon < 32 ? doc.left(colon) : QString();
}

QString presentationLabel(const QJsonObject &def) {
    static const QHash<QString, QString> table{
        {QStringLiteral("x"), QStringLiteral("Position")},     {QStringLiteral("scale"), QStringLiteral("Scale")},
        {QStringLiteral("rotation"), QStringLiteral("Rotation")}, {QStringLiteral("opacity"), QStringLiteral("Opacity")},
        {QStringLiteral("blend"), QStringLiteral("Blend")},     {QStringLiteral("gain"), QStringLiteral("Volume")},
        {QStringLiteral("pan"), QStringLiteral("Pan")},         {QStringLiteral("crop_l"), QStringLiteral("Left")},
        {QStringLiteral("crop_r"), QStringLiteral("Right")},    {QStringLiteral("crop_t"), QStringLiteral("Top")},
        {QStringLiteral("crop_b"), QStringLiteral("Bottom")},   {QStringLiteral("feather"), QStringLiteral("Feather")},
        {QStringLiteral("expansion"), QStringLiteral("Expansion")},
    };
    const QString name = def.value(QStringLiteral("name")).toString();
    if (table.contains(name) && !def.contains(QStringLiteral("group"))) return table.value(name);
    QString doc = def.value(QStringLiteral("doc")).toString().trimmed();
    if (const QString group = presentationGroup(def); !group.isEmpty()) doc = doc.mid(group.size() + 2).trimmed();
    if (!doc.isEmpty() && !doc.endsWith(QLatin1Char('.')) && doc.size() <= 28) return sentenceCase(doc);
    QString label = name;
    label.replace(QLatin1Char('_'), QLatin1Char(' '));
    return sentenceCase(label);
}

QString britishSpelling(const QString &text) {
    // The rest of the UI writes "colour"; the catalog writes "color".
    static const QRegularExpression word(QStringLiteral("\\b([Cc])olor"));
    QString out = text;
    out.replace(word, QStringLiteral("\\1olour"));
    return out;
}

namespace {

bool isAcronym(const QString &word) {
    int upper = 0;
    for (const QChar c : word) {
        if (c.isLower()) return false;
        if (c.isUpper()) ++upper;
    }
    return upper >= 2;
}

} // namespace

QString sentenceCase(const QString &text) {
    // "Edge Threshold" → "Edge threshold"; acronyms (LUT, RGB) and the
    // first word keep their capitals.
    QStringList words = text.split(QLatin1Char(' '));
    for (qsizetype index = 0; index < words.size(); ++index) {
        QString &word = words[index];
        if (word.isEmpty()) continue;
        if (index == 0) word[0] = word.at(0).toUpper();
        else if (!isAcronym(word)) word[0] = word.at(0).toLower();
    }
    return britishSpelling(words.join(QLatin1Char(' ')));
}

QString presentationEffectName(const QString &name) {
    // Title Case, as the Effects browser lists them ("Primary Color
    // Correction" for "Primary color correction."), small words lower.
    static const QStringList small{QStringLiteral("a"),  QStringLiteral("an"),  QStringLiteral("and"), QStringLiteral("the"),
                                   QStringLiteral("of"), QStringLiteral("to"),  QStringLiteral("in"),  QStringLiteral("on"),
                                   QStringLiteral("or"), QStringLiteral("for"), QStringLiteral("with"), QStringLiteral("by")};
    QString trimmed = name.trimmed();
    if (trimmed.endsWith(QLatin1Char('.'))) trimmed.chop(1);
    QStringList words = trimmed.split(QLatin1Char(' '));
    for (qsizetype index = 0; index < words.size(); ++index) {
        QString &word = words[index];
        if (word.isEmpty() || isAcronym(word)) continue;
        if (index > 0 && small.contains(word.toLower())) word = word.toLower();
        else word[0] = word.at(0).toUpper();
    }
    return britishSpelling(words.join(QLatin1Char(' ')));
}

QString withoutSharedWord(const QString &label, const QString &effectName) {
    // Under "Edge Glow", "Edge threshold" reads as "Threshold": the
    // effect's header already says "Edge".
    const qsizetype space = label.indexOf(QLatin1Char(' '));
    if (space <= 0 || space + 1 >= label.size()) return label;
    const QString first = label.left(space);
    const QStringList names = effectName.split(QLatin1Char(' '), Qt::SkipEmptyParts);
    const bool shared = std::any_of(names.begin(), names.end(), [&](const QString &word) { return word.compare(first, Qt::CaseInsensitive) == 0; });
    if (!shared) return label;
    QString rest = label.mid(space + 1).trimmed();
    // "Glow only" stays: "Only" alone says nothing.
    if (rest.isEmpty() || rest.compare(QStringLiteral("only"), Qt::CaseInsensitive) == 0) return label;
    rest[0] = rest.at(0).toUpper();
    return rest;
}

double gainToTravel(double db) {
    if (db >= 0.0) return 0.75 + 0.25 * std::min(db, 24.0) / 24.0;
    return 0.75 * std::pow(std::clamp((db + 96.0) / 96.0, 0.0, 1.0), 3.0);
}

double travelToGain(double travel) {
    travel = std::clamp(travel, 0.0, 1.0);
    if (travel >= 0.75) return (travel - 0.75) / 0.25 * 24.0;
    return std::cbrt(travel / 0.75) * 96.0 - 96.0;
}

} // namespace kerf
