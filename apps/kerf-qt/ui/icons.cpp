#include "icons.h"

#include <QByteArray>
#include <QHash>
#include <QList>
#include <QPainter>
#include <QPainterPathStroker>
#include <QPen>
#include <QRegularExpression>
#include <QtMath>
#include <QTransform>
#include <QVector>
#include <QXmlStreamAttributes>
#include <QXmlStreamReader>

#include <cmath>
#include <optional>

namespace kerf::icons {

namespace {

// ------------------------------------------------------------------ the table
// mockup-spec 16, verbatim: name, viewBox width and height, whether the
// design's round caps and joins apply (svg.i and .dch do; the playhead head is
// an inline SVG with SVG's defaults), and the markup inside the <symbol>.
struct Source {
    const char *name;
    int width;
    int height;
    bool round;
    const char *markup;
};

// clang-format off
constexpr Source kSources[] = {
    {"g-169", 16, 16, true, R"svg(<rect x="1" y="4" width="14" height="8" rx="1.5"/>)svg"},
    {"g-916", 16, 16, true, R"svg(<rect x="4.5" y="1.5" width="7" height="13" rx="1.5"/>)svg"},
    {"g-comments", 16, 16, true, R"svg(<path d="M2.5 3.5a1 1 0 0 1 1-1h9a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1H6l-3.5 2.5z"/>)svg"},
    {"g-mrect", 16, 16, true, R"svg(<rect x="2.5" y="3.5" width="11" height="9" rx="1" stroke-dasharray="2 1.5"/>)svg"},
    {"g-mell", 16, 16, true, R"svg(<ellipse cx="8" cy="8" rx="5.5" ry="4.5" stroke-dasharray="2 1.5"/>)svg"},
    {"g-mpen", 16, 16, true, R"svg(<path d="M3 12.5C5 6 11 10 13 3.5"/><circle cx="3" cy="12.5" r="1.25"/><circle cx="13" cy="3.5" r="1.25"/>)svg"},
    {"g-annot", 16, 16, true, R"svg(<path d="M10.5 2.5l3 3L6 13H3v-3z"/><path d="M9 4l3 3"/>)svg"},
    {"g-safe", 16, 16, true, R"svg(<rect x="1.5" y="2.5" width="13" height="11" rx="1.5"/><path d="M4.5 7V5.5h2M9.5 5.5h2V7M11.5 9v1.5h-2M6.5 10.5h-2V9"/>)svg"},
    {"g-dim", 16, 16, true, R"svg(<path fill="currentColor" fill-opacity=".35" stroke="none" fill-rule="evenodd" d="M3 2.5h10A1.5 1.5 0 0 1 14.5 4v8a1.5 1.5 0 0 1-1.5 1.5H3A1.5 1.5 0 0 1 1.5 12V4A1.5 1.5 0 0 1 3 2.5zM4.5 5.5v5h7v-5z"/><rect x="1.5" y="2.5" width="13" height="11" rx="1.5"/><rect x="4.5" y="5.5" width="7" height="5" rx=".5"/>)svg"},
    {"g-play", 16, 16, true, R"svg(<path d="M5 3.25v9.5L12.75 8z" fill="currentColor"/>)svg"},
    {"g-pause", 16, 16, true, R"svg(<rect x="4" y="3" width="2.75" height="10" rx=".75" fill="currentColor" stroke="none"/><rect x="9.25" y="3" width="2.75" height="10" rx=".75" fill="currentColor" stroke="none"/>)svg"},
    {"g-tostart", 16, 16, true, R"svg(<path d="M3.5 3.5v9"/><path d="M12.5 3.75v8.5L6.5 8z" fill="currentColor"/>)svg"},
    {"g-toend", 16, 16, true, R"svg(<path d="M12.5 3.5v9"/><path d="M3.5 3.75v8.5L9.5 8z" fill="currentColor"/>)svg"},
    {"g-prev", 16, 16, true, R"svg(<path d="M10 3.5 5.5 8l4.5 4.5"/>)svg"},
    {"g-next", 16, 16, true, R"svg(<path d="M6 3.5 10.5 8 6 12.5"/>)svg"},
    {"g-markin", 16, 16, true, R"svg(<path d="M8 2.5H4.5v11H8"/>)svg"},
    {"g-markout", 16, 16, true, R"svg(<path d="M8 2.5h3.5v11H8"/>)svg"},
    {"g-loop", 16, 16, true, R"svg(<path d="M11 2.5l2 2-2 2"/><path d="M13 4.5H5.5a3 3 0 0 0-3 3"/><path d="M5 13.5l-2-2 2-2"/><path d="M3 11.5h7.5a3 3 0 0 0 3-3"/>)svg"},
    {"g-import", 16, 16, true, R"svg(<path d="M8 2.5v7M5 6.5l3 3 3-3"/><path d="M2.5 10.5v2a1 1 0 0 0 1 1h9a1 1 0 0 0 1-1v-2"/>)svg"},
    {"g-undo", 16, 16, true, R"svg(<path d="M5.5 3.5 2.5 6.5l3 3"/><path d="M2.5 6.5h7a3.5 3.5 0 0 1 0 7H7"/>)svg"},
    {"g-redo", 16, 16, true, R"svg(<path d="M10.5 3.5l3 3-3 3"/><path d="M13.5 6.5h-7a3.5 3.5 0 0 0 0 7H9"/>)svg"},
    {"g-split", 16, 16, true, R"svg(<path d="M8 1.5v13"/><rect x="1.5" y="4.5" width="4.5" height="7" rx="1"/><rect x="10" y="4.5" width="4.5" height="7" rx="1"/>)svg"},
    {"g-delete", 16, 16, true, R"svg(<path d="M2.5 4.5h11M6.5 4.5V3a.5.5 0 0 1 .5-.5h2a.5.5 0 0 1 .5.5v1.5M4 4.5l.6 8.1a1 1 0 0 0 1 .9h4.8a1 1 0 0 0 1-.9L12 4.5"/>)svg"},
    {"g-settings", 24, 24, true, R"svg(<path stroke-width="2.25" d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle stroke-width="2.25" cx="12" cy="12" r="3"/>)svg"},
    {"g-chev", 16, 16, true, R"svg(<path d="M4.5 6.5 8 10l3.5-3.5"/>)svg"},
    {"g-chevl", 16, 16, true, R"svg(<path d="M9.5 4.5 6 8l3.5 3.5"/>)svg"},
    {"g-chevr", 16, 16, true, R"svg(<path d="M6.5 4.5 10 8l-3.5 3.5"/>)svg"},
    {"g-sel", 16, 16, true, R"svg(<path d="M3.5 2.5l3.75 10.5 1.6-4.4 4.4-1.6z"/>)svg"},
    {"g-razor", 16, 16, true, R"svg(<circle cx="4.5" cy="4.5" r="2"/><circle cx="4.5" cy="11.5" r="2"/><path d="M6.2 5.6 13.5 12.5M6.2 10.4 13.5 3.5"/>)svg"},
    {"g-ripple", 16, 16, true, R"svg(<path d="M5.5 2.5v11M5.5 2.5H8M5.5 13.5H8"/><path d="M13.5 8H9M11 5.5 13.5 8 11 10.5"/>)svg"},
    {"g-roll", 16, 16, true, R"svg(<path d="M8 2.5v11"/><path d="M5.5 5.5 3 8l2.5 2.5M10.5 5.5 13 8l-2.5 2.5"/>)svg"},
    {"g-slip", 16, 16, true, R"svg(<path d="M2.5 4.5v7M13.5 4.5v7"/><path d="M5 8h6M7 6 5 8l2 2M9 6l2 2-2 2"/>)svg"},
    {"g-slide", 16, 16, true, R"svg(<path d="M2.5 4.5v7M13.5 4.5v7"/><rect x="5.5" y="5.5" width="5" height="5" rx="1"/>)svg"},
    {"g-snap", 16, 16, true, R"svg(<path d="M3.5 2.5v5a4.5 4.5 0 0 0 9 0v-5"/><path d="M6.5 2.5v5a1.5 1.5 0 0 0 3 0v-5"/><path d="M3.5 5h3M9.5 5h3"/>)svg"},
    {"g-marker", 16, 16, true, R"svg(<path d="M4.5 2.5h7v11L8 11l-3.5 2.5z"/><path d="M8 5v3.5M6.25 6.75h3.5"/>)svg"},
    {"g-addtrack", 16, 16, true, R"svg(<rect x="2" y="3" width="12" height="4" rx="1"/><path d="M2 11h6M11 9v4M9 11h4"/>)svg"},
    {"g-ask", 16, 16, true, R"svg(<path d="M2.5 3.5a1 1 0 0 1 1-1h9a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1H6l-3.5 2.5z"/><path d="M8 5v4M6 7h4"/>)svg"},
    {"g-zout", 16, 16, true, R"svg(<path d="M3.5 8h9"/>)svg"},
    {"g-zin", 16, 16, true, R"svg(<path d="M8 3.5v9M3.5 8h9"/>)svg"},
    {"g-eye", 16, 16, true, R"svg(<path d="M1.5 8s2.4-4.5 6.5-4.5S14.5 8 14.5 8s-2.4 4.5-6.5 4.5S1.5 8 1.5 8z"/><circle cx="8" cy="8" r="2"/>)svg"},
    {"g-eyeoff", 16, 16, true, R"svg(<path d="M1.5 8s2.4-4.5 6.5-4.5S14.5 8 14.5 8s-2.4 4.5-6.5 4.5S1.5 8 1.5 8z"/><circle cx="8" cy="8" r="2"/><path d="M2.5 2.5l11 11"/>)svg"},
    {"g-lock", 16, 16, true, R"svg(<rect x="3.5" y="7" width="9" height="6.5" rx="1.5"/><path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2"/>)svg"},
    {"g-unlock", 16, 16, true, R"svg(<rect x="3.5" y="7" width="9" height="6.5" rx="1.5"/><path d="M5.5 7V5a2.5 2.5 0 0 1 4.9-.6"/>)svg"},
    {"g-synclock", 16, 16, true, R"svg(<path d="M6.5 9.5l3-3"/><path d="M7 4.5l1-1a2.83 2.83 0 0 1 4 4l-1 1M9 11.5l-1 1a2.83 2.83 0 0 1-4-4l1-1"/>)svg"},
    {"g-search", 16, 16, true, R"svg(<circle cx="7" cy="7" r="4.25"/><path d="M10.25 10.25 13.5 13.5"/>)svg"},
    {"g-sort", 16, 16, true, R"svg(<path d="M5 2.5v11M2.5 11 5 13.5 7.5 11M11 13.5v-11M8.5 5 11 2.5 13.5 5"/>)svg"},
    {"g-filter", 16, 16, true, R"svg(<path d="M2 4h12M4.5 8h7M6.5 12h3"/>)svg"},
    {"g-more", 16, 16, true, R"svg(<circle cx="3.5" cy="8" r="1" fill="currentColor" stroke="none"/><circle cx="8" cy="8" r="1" fill="currentColor" stroke="none"/><circle cx="12.5" cy="8" r="1" fill="currentColor" stroke="none"/>)svg"},
    {"g-close", 16, 16, true, R"svg(<path d="M4 4l8 8M12 4l-8 8"/>)svg"},
    {"g-send", 16, 16, true, R"svg(<path d="M8 13V3M3.5 7.5 8 3l4.5 4.5"/>)svg"},
    {"g-stop", 16, 16, true, R"svg(<rect x="4" y="4" width="8" height="8" rx="1.5" fill="currentColor" stroke="none"/>)svg"},
    {"g-at", 16, 16, true, R"svg(<circle cx="8" cy="8" r="2.5"/><path d="M10.5 5.5v3.25a1.75 1.75 0 0 0 3.5 0V8a6 6 0 1 0-2.35 4.77"/>)svg"},
    {"g-clock", 16, 16, true, R"svg(<circle cx="8" cy="8" r="5.75"/><path d="M8 5v3l2 1.25"/>)svg"},
    {"g-reply", 16, 16, true, R"svg(<path d="M6.5 4 2.5 8l4 4"/><path d="M2.5 8h6.5a4.5 4.5 0 0 1 4.5 4.5v.5"/>)svg"},
    {"g-pin", 16, 16, true, R"svg(<path d="M10 2.5 13.5 6l-2 .5-2.5 2.5.5 3-1 1L3 7.5l1-1 3 .5L9.5 4.5z"/><path d="M5.5 10.5 2.5 13.5"/>)svg"},
    {"g-check", 16, 16, true, R"svg(<path d="M3.5 8.5 6.5 11.5 12.5 4.5"/>)svg"},
    {"g-ring", 16, 16, true, R"svg(<circle cx="8" cy="8" r="6"/>)svg"},
    {"g-ringdone", 16, 16, true, R"svg(<circle cx="8" cy="8" r="6.5" fill="currentColor" stroke="none"/><path d="M5.25 8.25 7.1 10 10.75 6.25" stroke="#171717"/>)svg"},
    {"g-gauge", 16, 16, true, R"svg(<path d="M2.5 11a5.5 5.5 0 1 1 11 0"/><path d="M8 11l2.5-3.5"/>)svg"},
    {"g-kerf", 16, 16, true, R"svg(<path d="M3 3.5h4v5.5h2V3.5h4v9H3z" fill="currentColor" stroke="none"/>)svg"},
    {"g-video", 16, 16, true, R"svg(<rect x="2" y="3" width="12" height="10" rx="1.5"/><path d="M5 3v10M11 3v10M2 8h3M11 8h3"/>)svg"},
    {"g-audio", 16, 16, true, R"svg(<path d="M2.5 7v2M5 5v6M8 3v10M11 5.5v5M13.5 7v2"/>)svg"},
    {"g-info", 16, 16, true, R"svg(<circle cx="8" cy="8" r="5.75"/><path d="M8 7.25V11M8 5h.01"/>)svg"},
    {"g-kf", 16, 16, true, R"svg(<path d="M8 3.5 12.5 8 8 12.5 3.5 8z"/>)svg"},
    {"g-kff", 16, 16, true, R"svg(<path d="M8 3.5 12.5 8 8 12.5 3.5 8z" fill="currentColor"/>)svg"},
    {"g-reset", 16, 16, true, R"svg(<path d="M2.75 8a5.25 5.25 0 1 0 1.55-3.7L2.75 5.75"/><path d="M2.75 2.75v3h3"/>)svg"},
    {"g-grid", 16, 16, true, R"svg(<rect x="2.5" y="2.5" width="4.5" height="4.5" rx="1"/><rect x="9" y="2.5" width="4.5" height="4.5" rx="1"/><rect x="2.5" y="9" width="4.5" height="4.5" rx="1"/><rect x="9" y="9" width="4.5" height="4.5" rx="1"/>)svg"},
    {"g-menu", 16, 16, true, R"svg(<path d="M2.5 4.5h11M2.5 8h11M2.5 11.5h11"/>)svg"},
    {"g-list", 16, 16, true, R"svg(<path d="M2.5 4h11M2.5 8h11M2.5 12h11"/>)svg"},
    {"g-branch", 16, 16, true, R"svg(<circle cx="4.5" cy="3.5" r="1.5"/><circle cx="4.5" cy="12.5" r="1.5"/><circle cx="11.5" cy="5.5" r="1.5"/><path d="M4.5 5v6M11.5 7c0 2.5-3 2.5-5.5 4"/>)svg"},
    {"g-fx", 16, 16, true, R"svg(<path d="M2.5 4.5h11M2.5 11.5h11"/><circle cx="10" cy="4.5" r="1.5"/><circle cx="6" cy="11.5" r="1.5"/>)svg"},
    {"g-user", 16, 16, true, R"svg(<circle cx="8" cy="5.75" r="2.75"/><path d="M2.75 13.5a5.25 5.25 0 0 1 10.5 0"/>)svg"},
    {"g-cdr", 16, 16, true, R"svg(<path d="M3.5 2.5v5a2 2 0 0 0 2 2h7"/><path d="M10 7l2.5 2.5L10 12"/>)svg"},
    // Drawn outside the 16-unit set.
    {"playhead-head", 9, 12, false, R"svg(<path d="M.5 .5h8v7L4.5 11.5.5 7.5z" fill="#e5484d" stroke="#0e0e0e" stroke-width="1"/>)svg"},
    {"checkbox-check", 16, 16, true, R"svg(<path d="M3.5 8.5 6.5 11.5 12.5 4.5" stroke="#a8a8a8" stroke-width="1.75"/>)svg"},
};
// clang-format on

// A fill or stroke paint: none, currentColor, or a fixed colour.
struct Paint {
    enum Kind { None, Current, Fixed } kind = None;
    QColor color;
};

struct Shape {
    QPainterPath path;
    Paint fill;
    Paint stroke{Paint::Current, {}};
    qreal fillOpacity = 1;
    std::optional<qreal> strokeWidth;
    QVector<qreal> dashes; // in viewBox units
};

struct Icon {
    QSizeF viewBox;
    bool round = true;
    QList<Shape> shapes;
    QPainterPath geometry;
};

struct Registry {
    QStringList names;
    QHash<QString, Icon> icons;
    QStringList problems;
};

qreal number(const QXmlStreamAttributes &attributes, const char *name, qreal fallback = 0) {
    const QStringView value = attributes.value(QLatin1String(name));
    if (value.isEmpty()) return fallback;
    bool ok = false;
    const qreal parsed = value.toDouble(&ok);
    return ok ? parsed : fallback;
}

std::optional<Paint> paintAttribute(const QXmlStreamAttributes &attributes, const char *name) {
    if (!attributes.hasAttribute(QLatin1String(name))) return std::nullopt;
    const QString value = attributes.value(QLatin1String(name)).toString().trimmed();
    if (value == QLatin1String("none")) return Paint{Paint::None, {}};
    if (value == QLatin1String("currentColor")) return Paint{Paint::Current, {}};
    return Paint{Paint::Fixed, QColor(value)};
}

Icon readIcon(const Source &source, QStringList &problems) {
    Icon icon;
    icon.viewBox = QSizeF(source.width, source.height);
    icon.round = source.round;
    const QString name = QString::fromLatin1(source.name);
    QXmlStreamReader xml(QStringLiteral("<svg>") + QString::fromUtf8(source.markup) + QStringLiteral("</svg>"));
    while (!xml.atEnd()) {
        if (xml.readNext() != QXmlStreamReader::StartElement) continue;
        const QStringView tag = xml.name();
        if (tag == QLatin1String("svg")) continue;
        const QXmlStreamAttributes a = xml.attributes();
        Shape shape;
        if (tag == QLatin1String("path")) {
            QString error;
            shape.path = svg::parsePathData(a.value(QLatin1String("d")), &error);
            if (!error.isEmpty()) problems << name + QStringLiteral(": ") + error;
        } else if (tag == QLatin1String("rect")) {
            shape.path = svg::rectPath(number(a, "x"), number(a, "y"), number(a, "width"), number(a, "height"),
                                       number(a, "rx"), number(a, "ry"));
        } else if (tag == QLatin1String("circle")) {
            const qreal r = number(a, "r");
            shape.path = svg::ellipsePath(number(a, "cx"), number(a, "cy"), r, r);
        } else if (tag == QLatin1String("ellipse")) {
            shape.path = svg::ellipsePath(number(a, "cx"), number(a, "cy"), number(a, "rx"), number(a, "ry"));
        } else {
            problems << name + QStringLiteral(": unsupported element <") + tag.toString() + QLatin1Char('>');
            continue;
        }
        if (const auto fill = paintAttribute(a, "fill")) shape.fill = *fill;
        if (const auto stroke = paintAttribute(a, "stroke")) shape.stroke = *stroke;
        shape.fillOpacity = number(a, "fill-opacity", 1);
        if (a.hasAttribute(QLatin1String("stroke-width"))) shape.strokeWidth = number(a, "stroke-width", 1.5);
        if (a.value(QLatin1String("fill-rule")) == QLatin1String("evenodd")) shape.path.setFillRule(Qt::OddEvenFill);
        else shape.path.setFillRule(Qt::WindingFill);
        const QString dashes = a.value(QLatin1String("stroke-dasharray")).toString();
        for (const QString &part : dashes.split(QRegularExpression(QStringLiteral("[\\s,]+")), Qt::SkipEmptyParts))
            shape.dashes << part.toDouble();
        if (shape.dashes.size() % 2) shape.dashes += QVector<qreal>(shape.dashes); // SVG repeats an odd list
        if (shape.path.isEmpty()) problems << name + QStringLiteral(": an empty <") + tag.toString() + QLatin1Char('>');
        icon.geometry.addPath(shape.path);
        icon.shapes << shape;
    }
    if (xml.hasError()) problems << name + QStringLiteral(": ") + xml.errorString();
    if (icon.shapes.isEmpty()) problems << name + QStringLiteral(": no shapes");
    return icon;
}

const Registry &registry() {
    static const Registry built = [] {
        Registry r;
        for (const Source &source : kSources) {
            const QString name = QString::fromLatin1(source.name);
            r.names << name;
            r.icons.insert(name, readIcon(source, r.problems));
        }
        return r;
    }();
    return built;
}

const Icon *find(const QString &name) {
    const auto &icons = registry().icons;
    const auto it = icons.constFind(name);
    return it == icons.constEnd() ? nullptr : &*it;
}

QColor resolve(const Paint &paint, const QColor &current) {
    return paint.kind == Paint::Fixed ? paint.color : current;
}

QPen strokePen(const Icon &icon, const Shape &shape, const QColor &color, qreal px) {
    const qreal width = shape.strokeWidth.value_or(defaultStrokeWidth(px));
    QPen pen(color, width, Qt::SolidLine, icon.round ? Qt::RoundCap : Qt::FlatCap,
             icon.round ? Qt::RoundJoin : Qt::MiterJoin);
    pen.setMiterLimit(4); // SVG's default
    if (!shape.dashes.isEmpty() && width > 0) {
        QVector<qreal> pattern;
        for (const qreal dash : shape.dashes) pattern << dash / width; // Qt counts in pen widths
        pen.setDashPattern(pattern);
    }
    return pen;
}

// ------------------------------------------------------------ the path parser

class PathLexer {
public:
    explicit PathLexer(QStringView data) : m_data(data) {}

    void skipSeparators() {
        while (m_at < m_data.size() && (m_data[m_at].isSpace() || m_data[m_at] == QLatin1Char(','))) ++m_at;
    }
    bool atEnd() {
        skipSeparators();
        return m_at >= m_data.size();
    }
    bool atCommand() {
        skipSeparators();
        if (m_at >= m_data.size()) return false;
        const QChar c = m_data[m_at];
        return c.isLetter() && c != QLatin1Char('e') && c != QLatin1Char('E');
    }
    QChar takeCommand() { return m_data[m_at++]; }
    int position() const { return int(m_at); }

    bool number(qreal &out) {
        skipSeparators();
        const qsizetype begin = m_at;
        auto digit = [&](qsizetype i) { return i < m_data.size() && m_data[i].isDigit(); };
        qsizetype i = m_at;
        if (i < m_data.size() && (m_data[i] == QLatin1Char('+') || m_data[i] == QLatin1Char('-'))) ++i;
        bool digits = false;
        while (digit(i)) {
            ++i;
            digits = true;
        }
        if (i < m_data.size() && m_data[i] == QLatin1Char('.')) {
            ++i;
            while (digit(i)) {
                ++i;
                digits = true;
            }
        }
        if (!digits) return false;
        if (i < m_data.size() && (m_data[i] == QLatin1Char('e') || m_data[i] == QLatin1Char('E'))) {
            qsizetype j = i + 1;
            if (j < m_data.size() && (m_data[j] == QLatin1Char('+') || m_data[j] == QLatin1Char('-'))) ++j;
            if (digit(j)) {
                while (digit(j)) ++j;
                i = j;
            }
        }
        bool ok = false;
        out = m_data.sliced(begin, i - begin).toString().toDouble(&ok);
        if (!ok) return false;
        m_at = i;
        return true;
    }

    // An arc flag is one character, 0 or 1, and may run into what follows.
    bool flag(bool &out) {
        skipSeparators();
        if (m_at >= m_data.size()) return false;
        const QChar c = m_data[m_at];
        if (c != QLatin1Char('0') && c != QLatin1Char('1')) return false;
        out = c == QLatin1Char('1');
        ++m_at;
        return true;
    }

private:
    QStringView m_data;
    qsizetype m_at = 0;
};

int argumentCount(QChar command) {
    switch (command.toUpper().toLatin1()) {
    case 'Z': return 0;
    case 'H':
    case 'V': return 1;
    case 'M':
    case 'L':
    case 'T': return 2;
    case 'S':
    case 'Q': return 4;
    case 'C': return 6;
    case 'A': return 7;
    default: return -1;
    }
}

} // namespace

// ---------------------------------------------------------------- public API

QStringList names() { return registry().names; }

bool has(const QString &name) { return find(name) != nullptr; }

QSizeF viewBox(const QString &name) {
    const Icon *icon = find(name);
    return icon ? icon->viewBox : QSizeF();
}

QPainterPath path(const QString &name) {
    const Icon *icon = find(name);
    return icon ? icon->geometry : QPainterPath();
}

qreal defaultStrokeWidth(qreal px) { return qFuzzyCompare(px, qreal(14)) ? 1.7143 : 1.5; }

QPainterPath strokedPath(const QString &name, qreal px) {
    const Icon *icon = find(name);
    QPainterPath out;
    if (!icon) return out;
    out.setFillRule(Qt::WindingFill);
    for (const Shape &shape : icon->shapes) {
        if (shape.fill.kind != Paint::None) out.addPath(shape.path);
        if (shape.stroke.kind != Paint::None) {
            const QPen pen = strokePen(*icon, shape, Qt::black, px);
            QPainterPathStroker stroker(pen);
            out.addPath(stroker.createStroke(shape.path));
        }
    }
    return out;
}

QRectF iconBox(const QString &name, const QRectF &rect, qreal px) {
    QSizeF box = viewBox(name);
    if (box.isEmpty()) return {};
    box *= px / qMax(box.width(), box.height());
    return QRectF(rect.center() - QPointF(box.width() / 2, box.height() / 2), box);
}

void paint(QPainter &painter, const QString &name, const QRectF &rect, const QColor &color, qreal px) {
    const Icon *icon = find(name);
    if (!icon || px <= 0) return;
    const QRectF box = iconBox(name, rect, px);
    const qreal scale = box.width() / icon->viewBox.width();
    painter.save();
    painter.setRenderHint(QPainter::Antialiasing, true);
    painter.translate(box.topLeft());
    painter.scale(scale, scale);
    // SVG clips an icon to its viewport (a miter tip may poke past it).
    painter.setClipRect(QRectF(QPointF(0, 0), icon->viewBox), Qt::IntersectClip);
    for (const Shape &shape : icon->shapes) {
        if (shape.fill.kind != Paint::None) {
            QColor fill = resolve(shape.fill, color);
            fill.setAlphaF(fill.alphaF() * float(shape.fillOpacity));
            painter.fillPath(shape.path, fill);
        }
        if (shape.stroke.kind != Paint::None) {
            painter.setBrush(Qt::NoBrush);
            painter.setPen(strokePen(*icon, shape, resolve(shape.stroke, color), px));
            painter.drawPath(shape.path);
        }
    }
    painter.restore();
}

QPixmap pixmap(const QString &name, const QColor &color, qreal px, qreal dpr) {
    QSizeF box = viewBox(name);
    if (box.isEmpty()) return {};
    box *= px / qMax(box.width(), box.height());
    const QSize logical(int(std::ceil(box.width())), int(std::ceil(box.height())));
    QPixmap out(logical * dpr);
    out.setDevicePixelRatio(dpr);
    out.fill(Qt::transparent);
    QPainter painter(&out);
    paint(painter, name, QRectF(QPointF(0, 0), QSizeF(logical)), color, px);
    return out;
}

QIcon icon(const QString &name, const IconStates &states, qreal px) {
    QIcon out;
    if (!has(name)) return out;
    for (const qreal dpr : {1.0, 2.0}) {
        out.addPixmap(pixmap(name, states.normal, px, dpr), QIcon::Normal, QIcon::Off);
        out.addPixmap(pixmap(name, states.hover, px, dpr), QIcon::Active, QIcon::Off);
        out.addPixmap(pixmap(name, states.on, px, dpr), QIcon::Normal, QIcon::On);
        out.addPixmap(pixmap(name, states.on, px, dpr), QIcon::Active, QIcon::On);
        out.addPixmap(pixmap(name, states.on, px, dpr), QIcon::Selected, QIcon::Off);
        out.addPixmap(pixmap(name, states.on, px, dpr), QIcon::Selected, QIcon::On);
        out.addPixmap(pixmap(name, states.disabled, px, dpr), QIcon::Disabled, QIcon::Off);
        out.addPixmap(pixmap(name, states.disabled, px, dpr), QIcon::Disabled, QIcon::On);
    }
    return out;
}

QStringList problems() { return registry().problems; }

// ------------------------------------------------------------ the SVG reader

namespace svg {

void arcTo(QPainterPath &path, QPointF from, qreal rx, qreal ry, qreal xAxisRotationDegrees, bool largeArc,
           bool sweep, QPointF to) {
    if (from == to) return;
    rx = std::abs(rx);
    ry = std::abs(ry);
    if (rx == 0 || ry == 0) {
        path.lineTo(to);
        return;
    }
    const qreal phi = qDegreesToRadians(xAxisRotationDegrees);
    const qreal cosPhi = std::cos(phi), sinPhi = std::sin(phi);
    // F.6.5.1: the endpoints in the ellipse's own axes.
    const qreal dx2 = (from.x() - to.x()) / 2, dy2 = (from.y() - to.y()) / 2;
    const qreal x1 = cosPhi * dx2 + sinPhi * dy2;
    const qreal y1 = -sinPhi * dx2 + cosPhi * dy2;
    // F.6.6: radii too small for the chord grow until the arc just fits.
    const qreal lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if (lambda > 1) {
        const qreal grow = std::sqrt(lambda);
        rx *= grow;
        ry *= grow;
    }
    // F.6.5.2: the centre in the ellipse's axes.
    const qreal rx2 = rx * rx, ry2 = ry * ry;
    const qreal numerator = rx2 * ry2 - rx2 * y1 * y1 - ry2 * x1 * x1;
    const qreal denominator = rx2 * y1 * y1 + ry2 * x1 * x1;
    qreal coefficient = denominator > 0 ? std::sqrt(qMax<qreal>(0, numerator / denominator)) : 0;
    if (largeArc == sweep) coefficient = -coefficient;
    const qreal cx1 = coefficient * rx * y1 / ry;
    const qreal cy1 = -coefficient * ry * x1 / rx;
    // F.6.5.3: back to user space.
    const qreal cx = cosPhi * cx1 - sinPhi * cy1 + (from.x() + to.x()) / 2;
    const qreal cy = sinPhi * cx1 + cosPhi * cy1 + (from.y() + to.y()) / 2;
    // F.6.5.5-6: the start angle and the sweep.
    auto angle = [](qreal ux, qreal uy, qreal vx, qreal vy) { return std::atan2(ux * vy - uy * vx, ux * vx + uy * vy); };
    const qreal theta = angle(1, 0, (x1 - cx1) / rx, (y1 - cy1) / ry);
    qreal delta = angle((x1 - cx1) / rx, (y1 - cy1) / ry, (-x1 - cx1) / rx, (-y1 - cy1) / ry);
    if (!sweep && delta > 0) delta -= 2 * M_PI;
    else if (sweep && delta < 0) delta += 2 * M_PI;

    auto point = [&](qreal a) {
        const qreal c = std::cos(a), s = std::sin(a);
        return QPointF(cx + rx * c * cosPhi - ry * s * sinPhi, cy + rx * c * sinPhi + ry * s * cosPhi);
    };
    auto tangent = [&](qreal a) {
        const qreal c = std::cos(a), s = std::sin(a);
        return QPointF(-rx * s * cosPhi - ry * c * sinPhi, -rx * s * sinPhi + ry * c * cosPhi);
    };
    const int segments = qMax(1, int(std::ceil(std::abs(delta) / (M_PI / 2) - 1e-9)));
    const qreal step = delta / segments;
    const qreal k = 4.0 / 3.0 * std::tan(step / 4);
    qreal a = theta;
    for (int i = 0; i < segments; ++i) {
        const qreal b = a + step;
        const QPointF end = i == segments - 1 ? to : point(b);
        path.cubicTo(point(a) + k * tangent(a), point(b) - k * tangent(b), end);
        a = b;
    }
}

QPainterPath rectPath(qreal x, qreal y, qreal width, qreal height, qreal rx, qreal ry) {
    QPainterPath out;
    if (width <= 0 || height <= 0) return out;
    if (ry <= 0) ry = rx;
    if (rx <= 0) rx = ry;
    rx = qBound<qreal>(0, rx, width / 2);
    ry = qBound<qreal>(0, ry, height / 2);
    out.moveTo(x + rx, y);
    out.lineTo(x + width - rx, y);
    if (rx > 0) arcTo(out, QPointF(x + width - rx, y), rx, ry, 0, false, true, QPointF(x + width, y + ry));
    out.lineTo(x + width, y + height - ry);
    if (rx > 0)
        arcTo(out, QPointF(x + width, y + height - ry), rx, ry, 0, false, true, QPointF(x + width - rx, y + height));
    out.lineTo(x + rx, y + height);
    if (rx > 0) arcTo(out, QPointF(x + rx, y + height), rx, ry, 0, false, true, QPointF(x, y + height - ry));
    out.lineTo(x, y + ry);
    if (rx > 0) arcTo(out, QPointF(x, y + ry), rx, ry, 0, false, true, QPointF(x + rx, y));
    out.closeSubpath();
    return out;
}

QPainterPath ellipsePath(qreal cx, qreal cy, qreal rx, qreal ry) {
    QPainterPath out;
    if (rx <= 0 || ry <= 0) return out;
    // Four quarter arcs, clockwise on screen from the rightmost point.
    const QPointF points[] = {{cx + rx, cy}, {cx, cy + ry}, {cx - rx, cy}, {cx, cy - ry}, {cx + rx, cy}};
    out.moveTo(points[0]);
    for (int i = 0; i < 4; ++i) arcTo(out, points[i], rx, ry, 0, false, true, points[i + 1]);
    out.closeSubpath();
    return out;
}

QPainterPath parsePathData(QStringView data, QString *error) {
    QPainterPath out;
    if (error) error->clear();
    auto fail = [&](const QString &why, int at) {
        if (error) *error = QStringLiteral("path data at %1: %2").arg(at).arg(why);
        return out;
    };
    PathLexer lexer(data);
    QChar command;
    QPointF current, start;
    QPointF lastCubic, lastQuad; // the previous segment's second control point
    QChar previous;
    while (!lexer.atEnd()) {
        if (lexer.atCommand()) {
            command = lexer.takeCommand();
            if (argumentCount(command) < 0) return fail(QStringLiteral("unknown command ") + command, lexer.position());
        } else if (command.isNull()) {
            return fail(QStringLiteral("a number before any command"), lexer.position());
        } else if (argumentCount(command) == 0) {
            return fail(QStringLiteral("a number after Z"), lexer.position());
        }
        const bool relative = command.isLower();
        const char upper = command.toUpper().toLatin1();
        const QPointF base = relative ? current : QPointF();
        qreal v[7] = {};
        bool flags[2] = {};
        const int count = argumentCount(command);
        for (int i = 0; i < count; ++i) {
            bool ok;
            if (upper == 'A' && (i == 3 || i == 4)) {
                ok = lexer.flag(flags[i - 3]);
                v[i] = flags[i - 3];
            } else {
                ok = lexer.number(v[i]);
            }
            if (!ok) return fail(QStringLiteral("expected %1 values for ").arg(count) + command, lexer.position());
        }
        const char before = previous.toUpper().toLatin1();
        switch (upper) {
        case 'Z':
            out.closeSubpath();
            current = start;
            break;
        case 'M':
            current = base + QPointF(v[0], v[1]);
            start = current;
            out.moveTo(current);
            command = relative ? QLatin1Char('l') : QLatin1Char('L'); // repeats are lines
            break;
        case 'L':
            current = base + QPointF(v[0], v[1]);
            out.lineTo(current);
            break;
        case 'H':
            current.setX((relative ? current.x() : 0) + v[0]);
            out.lineTo(current);
            break;
        case 'V':
            current.setY((relative ? current.y() : 0) + v[0]);
            out.lineTo(current);
            break;
        case 'C': {
            const QPointF c1 = base + QPointF(v[0], v[1]), c2 = base + QPointF(v[2], v[3]);
            current = base + QPointF(v[4], v[5]);
            out.cubicTo(c1, c2, current);
            lastCubic = c2;
            break;
        }
        case 'S': {
            const QPointF c1 = (before == 'C' || before == 'S') ? 2 * current - lastCubic : current;
            const QPointF c2 = base + QPointF(v[0], v[1]);
            current = base + QPointF(v[2], v[3]);
            out.cubicTo(c1, c2, current);
            lastCubic = c2;
            break;
        }
        case 'Q': {
            const QPointF c = base + QPointF(v[0], v[1]);
            current = base + QPointF(v[2], v[3]);
            out.quadTo(c, current);
            lastQuad = c;
            break;
        }
        case 'T': {
            const QPointF c = (before == 'Q' || before == 'T') ? 2 * current - lastQuad : current;
            current = base + QPointF(v[0], v[1]);
            out.quadTo(c, current);
            lastQuad = c;
            break;
        }
        case 'A': {
            const QPointF to = base + QPointF(v[5], v[6]);
            arcTo(out, current, v[0], v[1], v[2], flags[0], flags[1], to);
            current = to;
            break;
        }
        }
        previous = QChar::fromLatin1(upper);
    }
    return out;
}

} // namespace svg

} // namespace kerf::icons
