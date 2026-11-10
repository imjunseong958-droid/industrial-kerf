#include "editor_common.h"
#include "design_tokens.h"
#include "editor_theme.h"
#include "motion_path.h"

#include <QAbstractItemView>
#include <QBrush>
#include <QApplication>
#include <QComboBox>
#include <QCoreApplication>
#include <QDir>
#include <QFileInfo>
#include <QFocusEvent>
#include <QFontDatabase>
#include <QHash>
#include <QImage>
#include <QJsonDocument>
#include <QLineEdit>
#include <QMenu>
#include <QPlainTextEdit>
#include <QPointer>
#include <QPolygonF>
#include <QProxyStyle>
#include <QPushButton>
#include <QRegularExpression>
#include <QStyleOptionMenuItem>
#include <QTemporaryDir>
#include <QTextEdit>
#include <QtMath>
#include <QToolButton>

#include <algorithm>
#include <array>
#include <cmath>
#include <limits>

namespace kerf {

namespace {

// Where a pop-up row's text starts, past the current choice's check.
constexpr int kPopupTextLeft = 22;
// A pop-up row is a menu item: 28pt (mockup-spec 0.5).
constexpr int kPopupRowHeight = 28;

} // namespace

bool nearerScrubFrame(double shownTime, bool hasShown, double frameTime, double target, double fps) {
    if (!hasShown) return true;
    const double slack = 0.5 / std::max(1.0, fps);
    return std::abs(frameTime - target) <= std::abs(shownTime - target) + slack;
}

QString validateInboundMcpConfig(const QByteArray &raw) {
    QJsonParseError error;
    const QJsonDocument document = QJsonDocument::fromJson(raw, &error);
    if (error.error != QJsonParseError::NoError)
        return QStringLiteral("Invalid JSON at offset %1: %2").arg(error.offset).arg(error.errorString());
    if (!document.isObject()) return QStringLiteral("The JSON root must be an object.");
    const QJsonValue servers = document.object().value(QStringLiteral("mcpServers"));
    if (!servers.isObject()) return QStringLiteral("mcpServers must be an object.");
    const QJsonObject object = servers.toObject();
    for (auto it = object.begin(); it != object.end(); ++it) {
        if (!it.value().isObject()) return QStringLiteral("Each server must be an object.");
        const QJsonObject server = it.value().toObject();
        const QJsonValue command = server.value(QStringLiteral("command"));
        if (!command.isString() || command.toString().trimmed().isEmpty())
            return QStringLiteral("Each server needs a nonempty local command string. URL-only remote servers are not supported in Kerf chat.");
        for (const QString &field : {QStringLiteral("args"), QStringLiteral("tools")}) {
            if (!server.contains(field) || (field == QStringLiteral("tools") && server.value(field).isNull())) continue;
            if (!server.value(field).isArray()) return QStringLiteral("%1 must be an array of strings.").arg(field);
            for (const QJsonValue &value : server.value(field).toArray())
                if (!value.isString()) return QStringLiteral("%1 must contain only strings.").arg(field);
        }
        if (server.contains(QStringLiteral("env"))) {
            if (!server.value(QStringLiteral("env")).isObject()) return QStringLiteral("env must be an object of string values.");
            const QJsonObject env = server.value(QStringLiteral("env")).toObject();
            for (auto entry = env.begin(); entry != env.end(); ++entry)
                if (!entry.value().isString()) return QStringLiteral("env must contain only string values.");
        }
    }
    return {};
}

QString mcpExecutable(const QString &projectDirectory) {
    const QString override = qEnvironmentVariable("KERF_MCP_BIN");
    if (!override.isEmpty()) {
        return QFileInfo(override).isRelative() && !override.contains(QLatin1Char('/'))
            && !override.contains(QLatin1Char('\\'))
            ? override : QFileInfo(override).absoluteFilePath();
    }
#ifdef Q_OS_WIN
    const QString binary = QStringLiteral("kerf-mcp.exe");
#else
    const QString binary = QStringLiteral("kerf-mcp");
#endif
    QStringList candidates{QDir(QCoreApplication::applicationDirPath()).filePath(binary)};
    const QStringList roots{QCoreApplication::applicationDirPath(), QDir::currentPath(),
                            QFileInfo(QString::fromUtf8(__FILE__)).absolutePath(), projectDirectory};
    for (const QString &root : roots) {
        QDir directory(root);
        do {
            if (directory.exists(QStringLiteral("crates/surfaces/kerf-mcp/Cargo.toml"))) {
                candidates << directory.filePath(QStringLiteral("target/release/") + binary)
                           << directory.filePath(QStringLiteral("target/debug/") + binary);
                break;
            }
        } while (directory.cdUp());
    }
    for (const QString &candidate : candidates) {
        const QFileInfo info(candidate);
        if (info.isFile() && info.isExecutable()) return info.absoluteFilePath();
    }
    return QStringLiteral("kerf-mcp");
}

QString prettyTime(double seconds) {
    // Rounded to hundredths first, so 59.996 carries to 01:00.00 rather
    // than showing 00:59.100.
    const long long hundredths = std::llround(std::max(0.0, seconds) * 100.0);
    return QStringLiteral("%1:%2.%3")
        .arg(hundredths / 6000, 2, 10, QLatin1Char('0'))
        .arg(hundredths / 100 % 60, 2, 10, QLatin1Char('0'))
        .arg(hundredths % 100, 2, 10, QLatin1Char('0'));
}

QString pathWithToolFolders(const QString &path, const QString &home) {
    QStringList folders = path.split(QLatin1Char(':'), Qt::SkipEmptyParts);
    for (const QString &folder : {QStringLiteral("/opt/homebrew/bin"), QStringLiteral("/opt/homebrew/sbin"),
                                  QStringLiteral("/usr/local/bin"), home + QStringLiteral("/.local/bin")})
        if (!folders.contains(folder)) folders.append(folder);
    return folders.join(QLatin1Char(':'));
}

QString timelineTimecode(double seconds, double fps) {
    const double rate = std::max(1.0, fps);
    const int frameRate = std::max(1, static_cast<int>(std::lround(rate)));
    const long long rounded = std::llround(std::max(0.0, seconds) * rate);
    const int frames = static_cast<int>(rounded % frameRate);
    long long whole = rounded / frameRate;
    const int secs = static_cast<int>(whole % 60);
    whole /= 60;
    const int mins = static_cast<int>(whole % 60);
    const int hours = static_cast<int>(whole / 60);
    return QStringLiteral("%1:%2:%3:%4")
        .arg(hours, 2, 10, QLatin1Char('0'))
        .arg(mins, 2, 10, QLatin1Char('0'))
        .arg(secs, 2, 10, QLatin1Char('0'))
        .arg(frames, 2, 10, QLatin1Char('0'));
}

RulerTicks rulerTicks(double pixelsPerSecond, double fps) {
    const double rate = std::max(1.0, fps);
    const qint64 perSecond = std::max<qint64>(1, std::llround(rate));
    // Ten minor ticks 8 apart is the least a label step may span.
    const double least = 80.0 / std::max(pixelsPerSecond, 1e-6);
    if (least <= 1.0) {
        // Labels stop at one a second; further in, only the ticks get finer.
        const double perFrame = std::max(pixelsPerSecond, 1e-6) / rate;
        qint64 minor = 1;
        while (perSecond % minor != 0 || double(minor) * perFrame < 8.0) ++minor;
        // The first whole part of the second at least five minor ticks long.
        qint64 major = minor * 5;
        while (major < perSecond && perSecond % major != 0) major += minor;
        return {minor, std::min(major, perSecond), perSecond};
    }
    qint64 seconds = 0;
    for (const qint64 step : {1, 2, 4, 8, 15, 30, 60, 120, 300, 600, 1200, 1800, 3600}) {
        if (double(step) >= least) {
            seconds = step;
            break;
        }
    }
    if (seconds == 0) seconds = 3600 * qint64(std::ceil(least / 3600.0));
    const qint64 label = seconds * perSecond;
    // The most minor ticks the label step divides into whole frames.
    for (const qint64 parts : {10, 8, 5, 4, 2}) {
        if (label % parts == 0) return {label / parts, parts % 2 == 0 ? label / 2 : label, label};
    }
    return {label, label, label};
}

QString rulerLabel(qint64 frame, double fps) {
    const int frameRate = std::max(1, static_cast<int>(std::lround(std::max(1.0, fps))));
    const qint64 seconds = std::max<qint64>(0, frame) / frameRate;
    const int hours = static_cast<int>(seconds / 3600);
    const int minutes = static_cast<int>((seconds / 60) % 60);
    const int secs = static_cast<int>(seconds % 60);
    return hours > 0 ? QStringLiteral("%1:%2:%3").arg(hours).arg(minutes, 2, 10, QLatin1Char('0')).arg(secs, 2, 10, QLatin1Char('0'))
                     : QStringLiteral("%1:%2").arg(minutes, 2, 10, QLatin1Char('0')).arg(secs, 2, 10, QLatin1Char('0'));
}

QString clipName(const QJsonObject &clip) {
    if (clip.value(QStringLiteral("name")).isString()) {
        return clip.value(QStringLiteral("name")).toString();
    }
    const QString id = clip.value(QStringLiteral("id")).toString();
    return id.isEmpty() ? QStringLiteral("Untitled clip") : id.left(18);
}

namespace {

// A copy's stem without the "-<hash>" media.rs appends (the first eight hex
// digits of the content's SHA-256), as long as something is left.
QString withoutContentHash(const QString &stem) {
    static const QRegularExpression hash(QStringLiteral("^(.*\\S)\\s*-[0-9a-f]{8}$"));
    const QRegularExpressionMatch match = hash.match(stem);
    return match.hasMatch() ? match.captured(1) : stem;
}

} // namespace

QString sourceName(const QJsonObject &source) {
    const QString path = source.value(QStringLiteral("path")).toString();
    return path.isEmpty() ? QStringLiteral("Untitled media") : withoutContentHash(QFileInfo(path).completeBaseName());
}

QString sourceFileName(const QJsonObject &source) {
    const QString path = source.value(QStringLiteral("path")).toString();
    if (path.isEmpty()) return QStringLiteral("Untitled media");
    const QFileInfo info(path);
    const QString stem = withoutContentHash(info.completeBaseName());
    return info.suffix().isEmpty() ? stem : stem + QLatin1Char('.') + info.suffix();
}

QHash<QString, QString> sourceNames(const QJsonArray &sources) {
    const auto unique = [&sources](const std::function<QString(const QJsonObject &)> &name) {
        QHash<QString, int> uses;
        for (const QJsonValue &value : sources) ++uses[name(value.toObject())];
        return uses;
    };
    const QHash<QString, int> stems = unique(sourceName);
    const QHash<QString, int> files = unique(sourceFileName);
    QHash<QString, QString> names;
    for (const QJsonValue &value : sources) {
        const QJsonObject source = value.toObject();
        QString name = sourceName(source);
        if (stems.value(name) > 1) name = sourceFileName(source);
        if (files.value(name) > 1) name = QFileInfo(source.value(QStringLiteral("path")).toString()).fileName();
        names.insert(source.value(QStringLiteral("id")).toString(), name);
    }
    return names;
}

QFont monoFont(int pixelSize, QFont::Weight weight) {
    // The tokens' mono face (JetBrains Mono when bundled, else Menlo, else
    // the system's fixed font), tabular, at the size asked for.
    QFont font = tokens::font(tokens::TypeStyle{pixelSize, pixelSize, int(weight), true});
    font.setPixelSize(pixelSize);
    font.setWeight(weight);
    return font;
}

QList<CanvasPreset> canvasPresets() {
    return {{"16:9", 1920, 1080}, {"9:16", 1080, 1920}};
}

QString canvasSizeLabel(const QSize &size) {
    QString name = QStringLiteral("Custom");
    for (const CanvasPreset &preset : canvasPresets())
        if (size == QSize(preset.width, preset.height)) name = QString::fromLatin1(preset.ratio);
    return QStringLiteral("%1 · %2\u2009×\u2009%3").arg(name).arg(size.width()).arg(size.height());
}

QSize resolvedCanvasSize(const QJsonObject &project, const QJsonArray &sources) {
    const auto validDimension = [](const QJsonValue &value) {
        const double number = value.toDouble();
        return std::isfinite(number) && number > 0.0;
    };
    const auto dimension = [](double value) {
        return static_cast<int>(std::round(std::clamp(
            value, 2.0, static_cast<double>(std::numeric_limits<int>::max()))));
    };
    if (validDimension(project.value(QStringLiteral("width"))) &&
        validDimension(project.value(QStringLiteral("height")))) {
        return QSize(dimension(project.value(QStringLiteral("width")).toDouble()),
                     dimension(project.value(QStringLiteral("height")).toDouble()));
    }
    QSize largest;
    double largestArea = 0.0;
    for (const auto &value : sources) {
        const QJsonObject source = value.toObject();
        if (!validDimension(source.value(QStringLiteral("width"))) ||
            !validDimension(source.value(QStringLiteral("height")))) continue;
        const double width = source.value(QStringLiteral("width")).toDouble();
        const double height = source.value(QStringLiteral("height")).toDouble();
        if (width * height > largestArea) {
            largestArea = width * height;
            largest = QSize(dimension(width), dimension(height));
        }
    }
    return largest.isValid() ? largest : QSize(1920, 1080);
}

QColor modelColor(const QString &hex) {
    QColor color(hex.left(7));
    if (hex.size() == 9) color.setAlpha(hex.mid(7).toInt(nullptr, 16));
    return color;
}

QString modelColorName(const QColor &color) {
    QString hex = color.name();
    if (color.alpha() != 255) hex += QStringLiteral("%1").arg(color.alpha(), 2, 16, QLatin1Char('0'));
    return hex;
}

bool isAudioTrack(const QJsonObject &track) {
    const QString type = track.value(QStringLiteral("type")).toString().toLower();
    return type.contains(QStringLiteral("audio")) || type.contains(QStringLiteral("sound"));
}

bool isCaptionTrack(const QJsonObject &track) {
    const QString type = track.value(QStringLiteral("type")).toString().toLower();
    return type.contains(QStringLiteral("subtitle")) || type.contains(QStringLiteral("caption"));
}

TrackBadge trackBadge(const QJsonObject &project, const QString &trackId) {
    int picture = 0;
    int sound = 0;
    for (const QJsonValue &value : project.value(QStringLiteral("tracks")).toArray()) {
        const QJsonObject track = value.toObject();
        const bool audio = isAudioTrack(track);
        const int number = audio ? ++sound : ++picture;
        if (track.value(QStringLiteral("id")).toString() != trackId) continue;
        QString name = track.value(QStringLiteral("name")).toString().trimmed();
        if (name.isEmpty())
            name = isCaptionTrack(track) ? QStringLiteral("Captions")
                                         : (audio ? QStringLiteral("Audio %1") : QStringLiteral("Video %1")).arg(number);
        return TrackBadge{(audio ? QStringLiteral("A") : QStringLiteral("V")) + QString::number(number), name};
    }
    return TrackBadge{trackId, QString()};
}

void frameInside(QPainter &p, const QRectF &rect, qreal thickness, const QColor &color) {
    p.fillRect(QRectF(rect.left(), rect.top(), rect.width(), thickness), color);
    p.fillRect(QRectF(rect.left(), rect.bottom() - thickness, rect.width(), thickness), color);
    p.fillRect(QRectF(rect.left(), rect.top() + thickness, thickness, rect.height() - 2 * thickness), color);
    p.fillRect(QRectF(rect.right() - thickness, rect.top() + thickness, thickness, rect.height() - 2 * thickness), color);
}

void drawUpDownChevrons(QPainter &painter, const QPointF &mid, const QColor &color, qreal reach) {
    painter.save();
    painter.setPen(QPen(color, 1.4, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    painter.setBrush(Qt::NoBrush);
    painter.drawPolyline(QPolygonF{mid + QPointF(-3, -1.5), mid + QPointF(0, -1.5 - reach), mid + QPointF(3, -1.5)});
    painter.drawPolyline(QPolygonF{mid + QPointF(-3, 1.5), mid + QPointF(0, 1.5 + reach), mid + QPointF(3, 1.5)});
    painter.restore();
}

namespace {
// A symbol `draw`n on a `size`-point grid as an image file the style sheet
// can name (with an @2x twin for Retina screens), written once per `key`
// per run; empty if it cannot be written.
QString glyphImagePath(const QString &key, const QSize &size, const std::function<void(QPainter &)> &draw) {
    static QTemporaryDir directory;
    static QHash<QString, QString> written;
    if (!directory.isValid()) return {};
    if (const auto found = written.constFind(key); found != written.cend()) return found.value();
    for (const int scale : {1, 2}) {
        QImage image(size * scale, QImage::Format_ARGB32_Premultiplied);
        image.fill(Qt::transparent);
        QPainter painter(&image);
        painter.setRenderHint(QPainter::Antialiasing);
        painter.scale(scale, scale);
        draw(painter);
        painter.end();
        const QString file = directory.filePath(key + (scale == 1 ? QStringLiteral(".png") : QStringLiteral("@2x.png")));
        if (!image.save(file)) return {};
    }
    const QString path = QDir::fromNativeSeparators(directory.filePath(key + QStringLiteral(".png")));
    written.insert(key, path);
    return path;
}

QString colorKey(const char *name, const QColor &color) {
    return QStringLiteral("%1-%2").arg(QLatin1String(name), color.name(QColor::HexArgb).mid(1));
}
} // namespace

QString popUpChevronsImagePath(const QColor &color) {
    // The pair, centred in its image.
    return glyphImagePath(colorKey("popup", color), QSize(10, 14),
                          [&color](QPainter &painter) { drawUpDownChevrons(painter, QPointF(5, 7), color, 3); });
}

QString checkMarkImagePath(const QColor &color) {
    return glyphImagePath(colorKey("check", color), QSize(14, 14), [&color](QPainter &painter) {
        painter.setPen(QPen(color, 1.8, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        painter.drawPolyline(QPolygonF{{3.6, 7.3}, {6.0, 9.7}, {10.6, 4.6}});
    });
}

void drawMenuCheck(QPainter &painter, const QPointF &at, const QColor &color) {
    painter.save();
    painter.setPen(QPen(color, 1.6, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    painter.setBrush(Qt::NoBrush);
    painter.drawPolyline(QPolygonF{at + QPointF(0, 0.5), at + QPointF(3, 3.5), at + QPointF(8, -3.5)});
    painter.restore();
}

QString menuCheckImagePath(const QColor &color) {
    // The menu's 18-point check column, centred on the row: the check sits
    // 7 points in, where a pop-up's rows draw theirs.
    return glyphImagePath(colorKey("menu-check", color), QSize(18, 14),
                          [&color](QPainter &painter) { drawMenuCheck(painter, QPointF(7, 7), color); });
}

QString menuChevronImagePath(const QColor &color) {
    return glyphImagePath(colorKey("menu-chevron", color), QSize(6, 10), [&color](QPainter &painter) {
        painter.setPen(QPen(color, 1.4, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        painter.drawPolyline(QPolygonF{{1.0, 1.0}, {5.0, 5.0}, {1.0, 9.0}});
    });
}

void StepperSpinBox::paintEvent(QPaintEvent *event) {
    QDoubleSpinBox::paintEvent(event);
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    drawUpDownChevrons(painter, QPointF(width() - 9.5, height() / 2.0), stepperColor(), 2.5);
}

QColor StepperSpinBox::stepperColor() const {
    return isEnabled() ? editorTheme().textSecondary : editorTheme().textDisabled;
}

PopupItemDelegate::PopupItemDelegate(QComboBox *combo) : QStyledItemDelegate(combo), m_combo(combo) {}

QSize PopupItemDelegate::sizeHint(const QStyleOptionViewItem &option, const QModelIndex &index) const {
    return {kPopupTextLeft + option.fontMetrics.horizontalAdvance(index.data(Qt::DisplayRole).toString()) + 16, kPopupRowHeight};
}

void PopupItemDelegate::paint(QPainter *painter, const QStyleOptionViewItem &option, const QModelIndex &index) const {
    const Colors c = colors();
    const bool selected = option.state & QStyle::State_Selected;
    const QRectF row(option.rect);
    painter->save();
    painter->setRenderHint(QPainter::Antialiasing);
    if (selected) {
        painter->setPen(Qt::NoPen);
        painter->setBrush(c.selection);
        painter->drawRoundedRect(row, 6, 6);
    }
    const QColor color = selected ? c.selectionText : c.text;
    if (index.row() == m_combo->currentIndex()) drawMenuCheck(*painter, QPointF(row.left() + 7, row.center().y()), color);
    painter->setFont(option.font);
    painter->setPen(color);
    painter->drawText(row.adjusted(kPopupTextLeft, 0, -8, 0), Qt::AlignLeft | Qt::AlignVCenter, index.data(Qt::DisplayRole).toString());
    painter->restore();
}

PopupItemDelegate::Colors PopupItemDelegate::colors() const {
    const EditorTheme &t = editorTheme();
    return {t.menuSelection, t.menuSelectionText, t.menuText};
}

namespace {

// Fusion with the design's flat metrics (mockup-spec 0.4, 0.5): controls
// 24pt tall (the style sheet makes toolbar ones 28), 6pt radii, 14pt check
// boxes, 16pt icons, a 16pt slider with a 10pt knob, no scroll bar width (the
// design hides them), and tooltips that wake after 400ms and come back at
// once within 300ms. Its push buttons (and check boxes) take focus from Tab
// alone, and its pop-up buttons' menu windows paint no panel of their own.
class EditorStyle final : public QProxyStyle {
public:
    EditorStyle() : QProxyStyle(QStringLiteral("Fusion")) {}

    int styleHint(StyleHint hint, const QStyleOption *option, const QWidget *widget, QStyleHintReturn *data) const override {
        switch (hint) {
        case SH_Button_FocusPolicy: return Qt::TabFocus;
        case SH_ToolTip_WakeUpDelay: return 400;
        case SH_ToolTip_FallAsleepDelay: return 300;
        case SH_ScrollBar_Transient: return 1;
        case SH_DialogButtonBox_ButtonsHaveIcons: return 0;
        case SH_Menu_FadeOutOnHide: return 0;
        default: return QProxyStyle::styleHint(hint, option, widget, data);
        }
    }

    int pixelMetric(PixelMetric metric, const QStyleOption *option, const QWidget *widget) const override {
        switch (metric) {
        case PM_IndicatorWidth:
        case PM_IndicatorHeight:
        case PM_ExclusiveIndicatorWidth:
        case PM_ExclusiveIndicatorHeight: return kCheckSize;
        case PM_SmallIconSize:
        case PM_ToolBarIconSize:
        case PM_ButtonIconSize:
        case PM_TabBarIconSize: return kIconSize;
        case PM_ScrollBarExtent: return 0;
        case PM_SliderThickness: return kSliderThickness;
        case PM_SliderLength:
        case PM_SliderControlThickness: return kSliderKnob;
        case PM_DefaultFrameWidth: return 1;
        case PM_MenuPanelWidth: return 1;
        case PM_MenuHMargin:
        case PM_MenuVMargin: return 4;
        case PM_ToolTipLabelFrameWidth: return 1;
        default: return QProxyStyle::pixelMetric(metric, option, widget);
        }
    }

    QSize sizeFromContents(ContentsType type, const QStyleOption *option, const QSize &contents, const QWidget *widget) const override {
        QSize size = QProxyStyle::sizeFromContents(type, option, contents, widget);
        switch (type) {
        case CT_PushButton:
        case CT_ComboBox:
        case CT_LineEdit:
        case CT_SpinBox: size.setHeight(std::max(size.height(), kControlHeight)); break;
        case CT_ToolButton: size = size.expandedTo(QSize(kControlHeight, kControlHeight)); break;
        case CT_MenuItem:
            if (const auto *item = qstyleoption_cast<const QStyleOptionMenuItem *>(option);
                item && item->menuItemType != QStyleOptionMenuItem::Separator)
                size.setHeight(std::max(size.height(), kMenuItemHeight));
            break;
        default: break;
        }
        return size;
    }

    void drawPrimitive(PrimitiveElement element, const QStyleOption *option, QPainter *painter, const QWidget *widget) const override {
        // A pop-up button's menu window paints nothing of its own (the style
        // sheet cannot reach it): the list in it, rounded by the style sheet,
        // is the whole menu.
        if (element == PE_PanelMenu && widget && widget->isWindow() && qobject_cast<const QComboBox *>(widget->parentWidget()))
            return;
        QProxyStyle::drawPrimitive(element, option, painter, widget);
    }

private:
    static constexpr int kControlHeight = 24;
    static constexpr int kMenuItemHeight = 28;
    static constexpr int kCheckSize = 14;
    static constexpr int kIconSize = 16;
    static constexpr int kSliderThickness = 16;
    static constexpr int kSliderKnob = 10;
};

// The ring around a focused field, or a button or pop-up focused from the
// keyboard: the design's focus outline, 2pt of the accent 1pt outside the
// control's bezel (mockup-spec 0.2). It sits in the control's parent, under
// the control, and follows it.
class FocusHalo final : public QWidget {
public:
    explicit FocusHalo(QWidget *control) : QWidget(control->parentWidget()), m_control(control) {
        setObjectName(QStringLiteral("focusHalo"));
        setAttribute(Qt::WA_TransparentForMouseEvents);
        setFocusPolicy(Qt::NoFocus);
        control->installEventFilter(this);
        place();
    }
    ~FocusHalo() override {
        if (m_control) m_control->removeEventFilter(this);
    }
    QWidget *control() const { return m_control; }

protected:
    bool eventFilter(QObject *watched, QEvent *event) override {
        if (watched == m_control) {
            switch (event->type()) {
            case QEvent::Move:
            case QEvent::Resize:
            case QEvent::Show:
            case QEvent::Hide: place(); break;
            default: break;
            }
        }
        return QWidget::eventFilter(watched, event);
    }

    void paintEvent(QPaintEvent *) override {
        if (!m_control) return;
        bool set = false;
        const qreal radius = m_control->property("bezelRadius").toReal(&set);
        QPainter painter(this);
        painter.setRenderHint(QPainter::Antialiasing);
        painter.setPen(QPen(editorTheme().focusHalo, kRing));
        painter.setBrush(Qt::NoBrush);
        // The pen's middle runs half its width inside the halo's edge, so
        // the ring ends kOffset outside the bezel.
        const qreal middle = kRing / 2.0;
        const qreal round = (set ? radius : 6.0) + kOffset + middle;
        painter.drawRoundedRect(QRectF(rect()).adjusted(middle, middle, -middle, -middle), round, round);
    }

private:
    static constexpr int kRing = 2;
    static constexpr int kOffset = 1;
    static constexpr int kWidth = kRing + kOffset;

    void place() {
        if (!m_control) return;
        setGeometry(m_control->geometry().adjusted(-kWidth, -kWidth, kWidth, kWidth));
        stackUnder(m_control);
        setVisible(m_control->isVisible());
    }

    QPointer<QWidget> m_control;
};

// Gives pop-up buttons their rows, pop-up windows and tooltips their round
// corners, and focused controls their ring, for the whole application.
class EditorControls final : public QObject {
public:
    using QObject::QObject;

protected:
    bool eventFilter(QObject *watched, QEvent *event) override {
        switch (event->type()) {
        case QEvent::Polish:
            if (auto *combo = qobject_cast<QComboBox *>(watched)) {
                // Qt's own rows (a menu item's or a list's) give way; a pop-up
                // that brings its own keeps them.
                const QAbstractItemDelegate *rows = combo->itemDelegate();
                if (!rows || rows->inherits("QComboMenuDelegate") || rows->inherits("QComboBoxDelegate"))
                    combo->setItemDelegate(new PopupItemDelegate(combo));
                roundCorners(combo->view()->window());
                // A pop-up button takes focus from Tab alone, as macOS's do.
                if (!combo->isEditable() && !ringsOwnFields(combo)) combo->setFocusPolicy(Qt::TabFocus);
            } else if (qobject_cast<QTextEdit *>(watched) || qobject_cast<QPlainTextEdit *>(watched)) {
                // The style sheet rounds a multi-line field's bezel by 3
                // points, and its ring follows (unless the field says otherwise).
                if (!watched->property("bezelRadius").isValid()) watched->setProperty("bezelRadius", 3);
            } else if (auto *menu = qobject_cast<QMenu *>(watched)) {
                roundCorners(menu);
                markColumns(menu);
            } else if (watched->inherits("QTipLabel")) {
                // The style sheet rounds a tooltip's card by 6pt.
                roundCorners(static_cast<QWidget *>(watched));
            }
            break;
        case QEvent::ActionAdded:
        case QEvent::ActionChanged:
        case QEvent::ActionRemoved:
            if (auto *menu = qobject_cast<QMenu *>(watched)) markColumns(menu);
            break;
        case QEvent::FocusIn: {
            auto *widget = qobject_cast<QWidget *>(watched);
            QWidget *field = widget ? typingField(widget) : nullptr;
            const auto *combo = qobject_cast<QComboBox *>(watched);
            const bool button = qobject_cast<QPushButton *>(watched) || qobject_cast<QToolButton *>(watched);
            if (!field && !button && !(combo && !combo->isEditable())) break;
            QWidget *control = field ? field : widget;
            if (!button && ringsOwnFields(control)) break;
            const Qt::FocusReason reason = static_cast<QFocusEvent *>(event)->reason();
            // A field being typed in is always ringed, as macOS rings it.
            // Tab (or a shortcut) rings a button or a pop-up, but not the
            // focus a window hands its first control as it opens; the ring
            // comes back with its window, or as a menu closes.
            const bool keyboard = control->window()->testAttribute(Qt::WA_KeyboardFocusChange);
            const bool returning = control == m_keyboardFocus &&
                                   (reason == Qt::ActiveWindowFocusReason || reason == Qt::PopupFocusReason);
            if ((field || keyboard || returning) && control->parentWidget()) {
                m_keyboardFocus = control;
                delete m_halo;
                m_halo = new FocusHalo(control);
            }
            break;
        }
        case QEvent::FocusOut:
            if (m_halo && watched == m_halo->control()) {
                const Qt::FocusReason reason = static_cast<QFocusEvent *>(event)->reason();
                if (reason != Qt::ActiveWindowFocusReason && reason != Qt::PopupFocusReason) m_keyboardFocus = nullptr;
                m_halo->hide();
                m_halo->deleteLater();
            }
            break;
        default:
            break;
        }
        return QObject::eventFilter(watched, event);
    }

private:
    // The style sheet rounds pop-up lists and menus; a translucent window
    // shows what is behind their corners, not squares of the window colour.
    // Set before the window is first shown, while it can still take it.
    static void roundCorners(QWidget *popup) {
        if (popup->testAttribute(Qt::WA_TranslucentBackground)) return;
#ifdef Q_OS_WIN
        // Windows gives only a frameless window an alpha channel.
        popup->setWindowFlag(Qt::FramelessWindowHint);
#endif
        popup->setAttribute(Qt::WA_TranslucentBackground);
    }

    // Whether `menu` shows checks, and whether any of its commands has a
    // symbol: Qt sets the rows' text past both, though menus show no
    // symbols. The style sheet then indents those rows less, so text starts
    // 22px in whatever a menu holds, as in macOS.
    static void markColumns(QMenu *menu) {
        bool checks = false;
        bool icons = false;
        for (const QAction *action : menu->actions()) {
            if (!action->isVisible() || action->isSeparator()) continue;
            checks = checks || action->isCheckable();
            icons = icons || !action->icon().isNull();
        }
        if (menu->property("checkColumn").toBool() == checks && menu->property("iconColumn").toBool() == icons) return;
        menu->setProperty("checkColumn", checks);
        menu->setProperty("iconColumn", icons);
        // The style sheet reads the properties as it polishes.
        if (menu->testAttribute(Qt::WA_WState_Polished)) {
            menu->style()->unpolish(menu);
            menu->style()->polish(menu);
        }
    }

    // The field `widget` types into, which is ringed while it has focus: a
    // line edit, a spin box (for its own line edit too), or a multi-line
    // field that is not read-only. A pop-up's line edit is no field of its own.
    static QWidget *typingField(QWidget *widget) {
        if (qobject_cast<QAbstractSpinBox *>(widget)) return widget;
        if (const auto *text = qobject_cast<QTextEdit *>(widget)) return text->isReadOnly() ? nullptr : widget;
        if (const auto *text = qobject_cast<QPlainTextEdit *>(widget)) return text->isReadOnly() ? nullptr : widget;
        if (!qobject_cast<QLineEdit *>(widget)) return nullptr;
        if (qobject_cast<QAbstractSpinBox *>(widget->parentWidget())) return widget->parentWidget();
        return qobject_cast<QComboBox *>(widget->parentWidget()) ? nullptr : widget;
    }

    // Whether `control` sits in a window or frame that shows its fields' and
    // pop-ups' focus itself (the launcher, the Assistant's composer).
    static bool ringsOwnFields(const QWidget *control) {
        for (const QWidget *frame = control; frame; frame = frame->parentWidget())
            if (frame->property("ringsOwnFields").toBool()) return true;
        return false;
    }

    QPointer<QWidget> m_keyboardFocus;
    QPointer<FocusHalo> m_halo;
};

} // namespace

void useEditorStyle() {
    static EditorControls *controls = nullptr;
    QApplication::setStyle(new EditorStyle);
    // Menus show no symbols, as macOS's: a command's symbol belongs to the
    // toolbar. Qt's cocoa theme would put it, grey, before each row of the
    // context menus and the menu bar's menus, and leave it dark on the
    // highlighted row.
    QCoreApplication::setAttribute(Qt::AA_DontShowIconsInMenus);
    if (controls || !qApp) return;
    controls = new EditorControls(qApp);
    qApp->installEventFilter(controls);
}

void drawWaveformSymbol(QPainter &painter, const QPointF &center, qreal size, const QColor &color) {
    const qreal scale = size / 18.0;
    const qreal heights[] = {4, 10, 16, 8, 12, 5};
    QPainterPath bars;
    for (int bar = 0; bar < 6; ++bar) {
        const QPointF at = center + QPointF((2 + bar * 2.8 - 9) * scale, 0);
        bars.moveTo(at - QPointF(0, heights[bar] / 2 * scale));
        bars.lineTo(at + QPointF(0, heights[bar] / 2 * scale));
    }
    painter.save();
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(QPen(color, 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    painter.setBrush(Qt::NoBrush);
    painter.drawPath(bars);
    painter.restore();
}

QPixmap glyphPixmap(const QSize &points, const std::function<void(QPainter &, const QColor &)> &draw, const QColor &color) {
    QPixmap image(points * 2);
    image.setDevicePixelRatio(2);
    image.fill(Qt::transparent);
    QPainter painter(&image);
    painter.setRenderHint(QPainter::Antialiasing);
    draw(painter, color);
    painter.end();
    return image;
}

QIcon glyphIcon(const QSize &points, const std::function<void(QPainter &, const QColor &)> &draw) {
    QIcon icon(glyphPixmap(points, draw, editorTheme().glyph));
    icon.addPixmap(glyphPixmap(points, draw, editorTheme().glyphDisabled), QIcon::Disabled);
    return icon;
}

QIcon disclosureIcon(bool open, const QColor &color) {
    QPixmap pixmap(QSize(12, 12) * 2);
    pixmap.setDevicePixelRatio(2);
    pixmap.fill(Qt::transparent);
    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(QPen(color, 1.3, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    if (open) painter.drawPolyline(QPolygonF{{0.75, 4.25}, {4.75, 8.25}, {8.75, 4.25}});
    else painter.drawPolyline(QPolygonF{{1.5, 2}, {5.5, 6}, {1.5, 10}});
    painter.end();
    QIcon icon(pixmap);
    icon.addPixmap(pixmap, QIcon::Disabled);
    return icon;
}

QPixmap warningTriangle(const QColor &color, int size) {
    QPixmap pixmap(QSize(size, size) * 2);
    pixmap.setDevicePixelRatio(2);
    pixmap.fill(Qt::transparent);
    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.scale(size / 14.0, size / 14.0);
    // The triangle, its corners rounded by a round-joined stroke of its own colour.
    const QPolygonF triangle{{7.0, 1.6}, {12.9, 12.0}, {1.1, 12.0}};
    painter.setPen(QPen(color, 1.8, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    painter.setBrush(color);
    painter.drawPolygon(triangle);
    // The mark, cut out so whatever is behind shows through it.
    painter.setCompositionMode(QPainter::CompositionMode_Clear);
    painter.setPen(QPen(Qt::black, 1.6, Qt::SolidLine, Qt::RoundCap));
    painter.drawLine(QPointF(7.0, 5.0), QPointF(7.0, 8.0));
    painter.setPen(Qt::NoPen);
    painter.setBrush(Qt::black);
    painter.drawEllipse(QPointF(7.0, 10.1), 0.95, 0.95);
    painter.end();
    return pixmap;
}

void blitLaneCache(QPainter &painter, const QPixmap &cache, const QRegion &clip, const QPoint &origin) {
    const qreal dpr = std::max<qreal>(1, cache.devicePixelRatio());
    painter.save();
    painter.setClipRegion(clip, Qt::IntersectClip);
    for (const QRect &logical : clip) {
        const QRectF source((logical.x() - origin.x()) * dpr, (logical.y() - origin.y()) * dpr, logical.width() * dpr,
                            logical.height() * dpr);
        painter.drawPixmap(QRectF(logical), cache, source);
    }
    painter.restore();
}

QIcon transportIcon(TransportGlyph glyph) {
    return glyphIcon(QSize(16, 16), [glyph](QPainter &painter, const QColor &color) {
        painter.setPen(Qt::NoPen);
        painter.setBrush(color);
        const auto triangle = [&painter](qreal base, qreal tip) {
            painter.drawPolygon(QPolygonF{{base, 3.5}, {tip, 8.0}, {base, 12.5}});
        };
        // A bracket opening towards `open` (+1 right, -1 left) with its spine at `x`.
        const auto bracket = [&painter, &color](qreal x, qreal open) {
            painter.save();
            painter.setPen(QPen(color, 1.6, Qt::SolidLine, Qt::FlatCap, Qt::MiterJoin));
            painter.setBrush(Qt::NoBrush);
            painter.drawPolyline(QPolygonF{{x + 3.6 * open, 2.8}, {x, 2.8}, {x, 13.2}, {x + 3.6 * open, 13.2}});
            painter.restore();
        };
        // An arrow coming down onto the track at `y`, from above.
        const auto arrow = [&painter, &color](qreal y) {
            painter.save();
            painter.setPen(QPen(color, 1.6, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
            painter.setBrush(Qt::NoBrush);
            painter.drawLine(QPointF(8.0, 1.8), QPointF(8.0, y - 1.2));
            painter.drawPolyline(QPolygonF{{5.2, y - 4.0}, {8.0, y - 1.2}, {10.8, y - 4.0}});
            painter.restore();
        };
        switch (glyph) {
        case TransportGlyph::Start: painter.drawRect(QRectF(3.0, 3.5, 1.6, 9.0)); triangle(13.0, 5.5); break;
        case TransportGlyph::PreviousFrame: triangle(10.5, 3.5); painter.drawRect(QRectF(11.5, 3.5, 1.6, 9.0)); break;
        case TransportGlyph::Play: triangle(4.5, 12.5); break;
        case TransportGlyph::Pause: painter.drawRect(QRectF(4.5, 3.5, 2.4, 9.0)); painter.drawRect(QRectF(9.1, 3.5, 2.4, 9.0)); break;
        case TransportGlyph::NextFrame: painter.drawRect(QRectF(2.9, 3.5, 1.6, 9.0)); triangle(5.5, 12.5); break;
        case TransportGlyph::End: triangle(3.0, 10.5); painter.drawRect(QRectF(11.4, 3.5, 1.6, 9.0)); break;
        // Marks: a bracket with a small tag pointing into the marked range.
        case TransportGlyph::MarkIn:
            bracket(4.3, 1);
            painter.drawPolygon(QPolygonF{{8.0, 5.0}, {12.0, 8.0}, {8.0, 11.0}});
            break;
        case TransportGlyph::MarkOut:
            bracket(11.7, -1);
            painter.drawPolygon(QPolygonF{{8.0, 5.0}, {4.0, 8.0}, {8.0, 11.0}});
            break;
        // Edits: the source comes down between two clips, pushing them
        // apart, or onto one clip, covering it.
        case TransportGlyph::Insert:
            arrow(10.5);
            painter.drawRoundedRect(QRectF(0.8, 11.0, 5.4, 3.6), 0.8, 0.8);
            painter.drawRoundedRect(QRectF(9.8, 11.0, 5.4, 3.6), 0.8, 0.8);
            break;
        case TransportGlyph::Overwrite:
            arrow(10.5);
            painter.drawRoundedRect(QRectF(0.8, 11.0, 14.4, 3.6), 0.8, 0.8);
            break;
        }
    });
}

void drawPenNib(QPainter &painter, const QPointF &center, const QColor &color) {
    // A pen's nib: a 10-point diagonal shaft ending in a 3-point solid tip
    // at the lower left.
    painter.save();
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(QPen(color, 1.25, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    painter.setBrush(Qt::NoBrush);
    painter.drawLine(center + QPointF(-2.4, 2.4), center + QPointF(4.6, -4.6));
    painter.setPen(Qt::NoPen);
    painter.setBrush(color);
    painter.drawPolygon(QPolygonF{center + QPointF(-5.0, 5.0), center + QPointF(-3.7, 1.4), center + QPointF(-1.4, 3.7)});
    painter.restore();
}

QIcon maskToolIcon(MaskTool tool) {
    const auto draw = [tool](QPainter &painter, const QColor &color) {
        painter.setPen(QPen(color, 1.25));
        painter.setBrush(Qt::NoBrush);
        switch (tool) {
        case MaskTool::None: break;
        case MaskTool::Rectangle: painter.drawRect(QRectF(2.5, 3.5, 11.0, 9.0)); break;
        case MaskTool::Ellipse: painter.drawEllipse(QRectF(2.0, 3.0, 12.0, 10.0)); break;
        case MaskTool::Pen: drawPenNib(painter, QPointF(8.0, 8.0), color); break;
        }
    };
    // Quiet at rest (secondary ink, as the headers they sit in), full ink
    // under the pointer, and the accent while chosen.
    QIcon icon(glyphPixmap(QSize(16, 16), draw, editorTheme().textSecondary));
    icon.addPixmap(glyphPixmap(QSize(16, 16), draw, editorTheme().text), QIcon::Active, QIcon::Off);
    icon.addPixmap(glyphPixmap(QSize(16, 16), draw, editorTheme().glyphDisabled), QIcon::Disabled, QIcon::Off);
    const QPixmap chosen = glyphPixmap(QSize(16, 16), draw, editorTheme().accentText);
    icon.addPixmap(chosen, QIcon::Normal, QIcon::On);
    icon.addPixmap(chosen, QIcon::Active, QIcon::On);
    icon.addPixmap(glyphPixmap(QSize(16, 16), draw, editorTheme().glyphDisabled), QIcon::Disabled, QIcon::On);
    return icon;
}

double propertyAt(const QJsonObject &props, const QString &name, double clipTime, double fallback) {
    const QJsonValue value = props.value(name);
    if (value.isDouble()) return value.toDouble();
    const QJsonArray keys = value.toArray();
    if (keys.isEmpty()) return fallback;
    const auto at = [&keys](int index) { return keys.at(index).toObject().value(QStringLiteral("at")).toDouble(); };
    const auto valueOf = [&keys](int index) { return keys.at(index).toObject().value(QStringLiteral("value")).toDouble(); };
    const auto interp = [&keys](int index) { return keys.at(index).toObject().value(QStringLiteral("interp")).toString(QStringLiteral("linear")); };
    if (clipTime <= at(0)) return valueOf(0);
    for (int index = 1; index < keys.size(); ++index) {
        if (clipTime > at(index)) continue;
        const double span = at(index) - at(index - 1);
        const double t = span > 0.0 ? (clipTime - at(index - 1)) / span : 1.0;
        return valueOf(index - 1) + (valueOf(index) - valueOf(index - 1)) * path::curve(interp(index - 1), t);
    }
    return valueOf(keys.size() - 1);
}

namespace {

// What a picture of the cropped size fills on the canvas, before the clip's own
// scale.
QSizeF fittedSize(const QSizeF &cropped, bool fit, const QSizeF &canvas) {
    if (!fit) return cropped;
    return {std::max(1.0, std::min(std::round(canvas.height() * cropped.width() / cropped.height()), canvas.width())),
            std::max(1.0, std::min(std::round(canvas.width() * cropped.height() / cropped.width()), canvas.height()))};
}

// A clip's geometry at `clipTime`: the compositor's `GEOMETRY`.
struct Geometry {
    double cropL = 0.0, cropR = 0.0, cropT = 0.0, cropB = 0.0;
    double scale = 1.0, scaleX = 1.0, scaleY = 1.0, rotation = 0.0;
    double anchorX = 0.0, anchorY = 0.0, x = 0.0, y = 0.0;
};

Geometry geometryAt(const QJsonObject &clip, double clipTime) {
    const QJsonObject props = clip.value(QStringLiteral("props")).toObject();
    const auto prop = [&](const char *name, double fallback) { return propertyAt(props, QString::fromLatin1(name), clipTime, fallback); };
    return {prop("crop_l", 0.0), prop("crop_r", 0.0), prop("crop_t", 0.0), prop("crop_b", 0.0), prop("scale", 1.0), prop("scale_x", 1.0),
            prop("scale_y", 1.0), prop("rotation", 0.0), prop("anchor_x", 0.0), prop("anchor_y", 0.0), prop("x", 0.0), prop("y", 0.0)};
}

// The same with its motion path added in (kerf-compose `geometry_numbers`): the
// path adds to where it would be, and `orient` to how it is turned.
Geometry movedGeometryAt(const QJsonObject &clip, const QSizeF &canvas, double clipTime) {
    Geometry geometry = geometryAt(clip, clipTime);
    const QJsonObject props = clip.value(QStringLiteral("props")).toObject();
    if (const auto points = path::parsePath(props.value(QStringLiteral("path")).toString())) {
        const auto on = path::MotionPath(*points, canvas.width() / std::max(1.0, canvas.height()))
                            .at(propertyAt(props, QStringLiteral("along"), clipTime, 0.0));
        geometry.x += on.x;
        geometry.y += on.y;
        if (props.value(QStringLiteral("orient")).toBool()) geometry.rotation += on.heading;
    }
    return geometry;
}

// Where a source of `picture` pixels lands on the canvas, in canvas pixels
// (kerf-compose `placed`).
QTransform placed(const Geometry &g, const QSizeF &picture, bool fit, const QSizeF &canvas) {
    const double sw = std::max(1.0, picture.width()), sh = std::max(1.0, picture.height());
    const double width = sw * std::max(1e-6, 1.0 - g.cropL - g.cropR), height = sh * std::max(1e-6, 1.0 - g.cropT - g.cropB);
    const QSizeF filled = fittedSize({width, height}, fit, canvas);
    const double fw = filled.width(), fh = filled.height();
    QTransform placement;
    placement.translate(canvas.width() / 2.0 + g.x * canvas.width() + g.anchorX * fw, canvas.height() / 2.0 + g.y * canvas.height() + g.anchorY * fh);
    placement.rotate(g.rotation);
    placement.scale(fw / width * g.scale * g.scaleX, fh / height * g.scale * g.scaleY);
    placement.translate(-(sw * g.cropL + width * (0.5 + g.anchorX)), -(sh * g.cropT + height * (0.5 + g.anchorY)));
    return placement;
}

// Fractions of the picture to canvas pixels, flat: the compositor's geometry
// (kerf-compose `Drawing::geometry`).
QTransform flatPlacement(const QJsonObject &clip, const QSizeF &picture, bool fit, const QSizeF &canvas, double clipTime, const QTransform &parents) {
    const QTransform source = QTransform::fromScale(std::max(1.0, picture.width()), std::max(1.0, picture.height()));
    return source * placed(movedGeometryAt(clip, canvas, clipTime), picture, fit, canvas) * parents;
}

// The nearest a camera sees, in frame heights, and what it sees at rest
// (kerf-compose `three_d.rs`).
constexpr double kNearest = 0.001;
constexpr double kRestingFov = 40.0;

// A point or a way in the compositor's world: x right, y down, and z away from
// the viewer, in canvas pixels, with 0, 0, 0 the middle of the canvas.
struct World {
    double x = 0.0, y = 0.0, z = 0.0;
    World operator+(const World &other) const { return {x + other.x, y + other.y, z + other.z}; }
    World operator-(const World &other) const { return {x - other.x, y - other.y, z - other.z}; }
    World operator*(double by) const { return {x * by, y * by, z * by}; }
};

double dot(const World &a, const World &b) { return a.x * b.x + a.y * b.y + a.z * b.z; }

World cross(const World &a, const World &b) {
    return {a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x};
}

// Turned about x by `pitch` and then about y by `yaw`, in degrees.
World turned(const World &a, double pitch, double yaw) {
    const double sp = std::sin(qDegreesToRadians(pitch)), cp = std::cos(qDegreesToRadians(pitch));
    const double sy = std::sin(qDegreesToRadians(yaw)), cy = std::cos(qDegreesToRadians(yaw));
    const double y = a.y * cp - a.z * sp, z = a.y * sp + a.z * cp;
    return {a.x * cy + z * sy, y, -a.x * sy + z * cy};
}

// What a placement is when there is nothing to place on: not invertible.
QTransform unseen() { return QTransform(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0); }

// How a sequence is seen at a moment (kerf-compose `Scene::at`).
struct Camera {
    World eye, right, down, forward;
    double focal = 1.0, width = 1.0, height = 1.0;

    // Where (a, b) of the plane `origin + a * across + b * below` lands, in canvas
    // pixels. Its third row is how far along the camera's way that is, in frame
    // heights, which is what makes behind it negative and inSight possible.
    QTransform sees(const World &origin, const World &across, const World &below) const {
        const World from = origin - eye;
        const World perpendicular = cross(across, below);
        const double area = std::sqrt(dot(perpendicular, perpendicular));
        // Edge on it is a line at most a pixel wide, and the compositor's rays miss it.
        if (area <= 0.0 || std::abs(dot(from, perpendicular)) / area < kNearest * height) return unseen();
        const World sideways = right * focal + forward * (width / 2.0);
        const World downwards = down * focal + forward * (height / 2.0);
        const auto row = [this](const World &of, const World &way) { return dot(of, way) / height; };
        return QTransform(row(across, sideways), row(across, downwards), row(across, forward),
                          row(below, sideways), row(below, downwards), row(below, forward),
                          row(from, sideways), row(from, downwards), row(from, forward));
    }
};

// `timeline`'s topmost camera showing at `at` seconds, or the default one.
Camera cameraAt(const QJsonObject &timeline, const QSizeF &canvas, double at) {
    // Times come as seconds in doubles, a hair off the frames they name.
    constexpr double kSameMoment = 1e-9;
    QJsonObject shown;
    for (const QJsonValue &track : timeline.value(QStringLiteral("tracks")).toArray()) {
        if (!track.toObject().value(QStringLiteral("enabled")).toBool(true)) continue;
        for (const QJsonValue &value : track.toObject().value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = value.toObject();
            if (clip.value(QStringLiteral("fx")).toString() != QStringLiteral("camera") || !clip.value(QStringLiteral("enabled")).toBool(true)) continue;
            const double start = clip.value(QStringLiteral("timelineStart")).toDouble();
            if (start - kSameMoment <= at && at < start + clip.value(QStringLiteral("duration")).toDouble() - kSameMoment) shown = clip;
        }
    }
    const QJsonObject props = shown.value(QStringLiteral("props")).toObject();
    const double clipTime = at - shown.value(QStringLiteral("timelineStart")).toDouble();
    const auto read = [&](const char *name, double otherwise) { return propertyAt(props, QString::fromLatin1(name), clipTime, otherwise); };
    Camera camera;
    camera.width = std::max(1.0, canvas.width());
    camera.height = std::max(1.0, canvas.height());
    const auto distance = [&camera](double fov) { return camera.height / 2.0 / std::tan(qDegreesToRadians(std::clamp(fov, 1.0, 170.0)) / 2.0); };
    camera.eye = {read("x", 0.0) * camera.width, read("y", 0.0) * camera.height, read("z", 0.0) * camera.height - distance(kRestingFov)};
    camera.focal = distance(read("fov", kRestingFov));
    const double pitch = read("tilt", 0.0), yaw = read("pan", 0.0), roll = qDegreesToRadians(read("roll", 0.0));
    camera.forward = turned({0.0, 0.0, 1.0}, pitch, yaw);
    const World levelRight = turned({1.0, 0.0, 0.0}, pitch, yaw);
    const World levelDown = cross(camera.forward, levelRight);
    camera.right = levelRight * std::cos(roll) + levelDown * std::sin(roll);
    camera.down = levelDown * std::cos(roll) - levelRight * std::sin(roll);
    return camera;
}

bool isInDepth(const QJsonObject &clip) { return clip.value(QStringLiteral("props")).toObject().value(QStringLiteral("three_d")).toBool(); }

// A clip's own numbers in depth at `clipTime`.
struct Depth {
    double z = 0.0, pitch = 0.0, yaw = 0.0;
};

Depth depthAt(const QJsonObject &clip, const QSizeF &canvas, double clipTime) {
    const QJsonObject props = clip.value(QStringLiteral("props")).toObject();
    return {propertyAt(props, QStringLiteral("z"), clipTime, 0.0) * std::max(1.0, canvas.height()), propertyAt(props, QStringLiteral("rotate_x"), clipTime, 0.0),
            propertyAt(props, QStringLiteral("rotate_y"), clipTime, 0.0)};
}

} // namespace

QTransform pathPlacement(const QJsonObject &clip, const QSizeF &picture, bool fit, const QSizeF &canvas, double clipTime,
                         const QTransform &parents, const QJsonObject &timeline) {
    const Geometry g = geometryAt(clip, clipTime);
    const double width = std::max(1.0, picture.width()) * std::max(1e-6, 1.0 - g.cropL - g.cropR);
    const double height = std::max(1.0, picture.height()) * std::max(1e-6, 1.0 - g.cropT - g.cropB);
    const QSizeF filled = fittedSize({width, height}, fit, canvas);
    const double cw = std::max(1.0, canvas.width()), ch = std::max(1.0, canvas.height());
    const QPointF origin(0.5 + g.x + g.anchorX * filled.width() / cw, 0.5 + g.y + g.anchorY * filled.height() / ch);
    const QTransform onCanvas = QTransform::fromTranslate(origin.x(), origin.y()) * QTransform::fromScale(cw, ch) * parents;
    const QTransform toShares = QTransform::fromScale(1.0 / cw, 1.0 / ch);
    if (!isInDepth(clip)) return onCanvas * toShares;
    // The path says where the anchor goes, and the clip turns about its middle.
    const QTransform flat = flatPlacement(clip, picture, fit, canvas, clipTime, parents);
    const QPointF anchor(g.cropL + (1.0 - g.cropL - g.cropR) * (0.5 + g.anchorX), g.cropT + (1.0 - g.cropT - g.cropB) * (0.5 + g.anchorY));
    const QPointF toMiddle = flat.map(QPointF(0.5, 0.5)) - flat.map(anchor);
    const Depth depth = depthAt(clip, canvas, clipTime);
    const World swung = turned({-toMiddle.x(), -toMiddle.y(), 0.0}, depth.pitch, depth.yaw) + World{toMiddle.x(), toMiddle.y(), 0.0};
    if (dot(swung, swung) > 1e-12 && clip.value(QStringLiteral("props")).toObject().value(QStringLiteral("orient")).toBool()) return unseen();
    const Camera camera = cameraAt(timeline, canvas, clip.value(QStringLiteral("timelineStart")).toDouble() + clipTime);
    const World plane = World{-cw / 2.0, -ch / 2.0, depth.z} + swung;
    return onCanvas * camera.sees(plane, {1.0, 0.0, 0.0}, {0.0, 1.0, 0.0}) * toShares;
}

QList<std::optional<QPointF>> sampledPath(const QJsonObject &clip, const QSizeF &picture, bool fit, const QSizeF &canvas, double clipTime,
                                          const QTransform &parents, const QJsonObject &timeline, int count) {
    QList<std::optional<QPointF>> places;
    const QJsonObject props = clip.value(QStringLiteral("props")).toObject();
    const Geometry g = geometryAt(clip, clipTime);
    const QPointF anchor(g.cropL + (1.0 - g.cropL - g.cropR) * (0.5 + g.anchorX), g.cropT + (1.0 - g.cropT - g.cropB) * (0.5 + g.anchorY));
    const double cw = std::max(1.0, canvas.width()), ch = std::max(1.0, canvas.height());
    for (int step = 0; step <= count; ++step) {
        QJsonObject there = props;
        there.insert(QStringLiteral("along"), double(step) / double(std::max(1, count)));
        QJsonObject moved = clip;
        moved.insert(QStringLiteral("props"), there);
        const QTransform placement = picturePlacement(moved, picture, fit, canvas, clipTime, parents, timeline);
        if (!placement.isInvertible() || !inSight(placement, anchor)) {
            places.append(std::nullopt);
            continue;
        }
        const QPointF on = placement.map(anchor);
        places.append(QPointF(on.x() / cw, on.y() / ch));
    }
    return places;
}

QJsonObject withPathPoints(const QJsonObject &clip, const QList<QPointF> &points) {
    QStringList parts;
    for (const QPointF &point : points) parts << QStringLiteral("%1,%2").arg(point.x(), 0, 'g', 17).arg(point.y(), 0, 'g', 17);
    QJsonObject props = clip.value(QStringLiteral("props")).toObject();
    props.insert(QStringLiteral("path"), parts.join(QLatin1Char(' ')));
    QJsonObject moved = clip;
    moved.insert(QStringLiteral("props"), props);
    return moved;
}

namespace {

// Where the anchor of `clip`, whose path is `points` said closely, is when at point `index` of it.
std::optional<QPointF> anchorAtPoint(const QJsonObject &clip, const QSizeF &picture, bool fit, const QSizeF &canvas, double clipTime, const QTransform &parents,
                                     const QJsonObject &timeline, const QList<QPointF> &points, int index) {
    const double cw = std::max(1.0, canvas.width()), ch = std::max(1.0, canvas.height());
    QJsonObject moved = withPathPoints(clip, points);
    QJsonObject props = moved.value(QStringLiteral("props")).toObject();
    props.insert(QStringLiteral("along"), path::MotionPath(points, cw / ch).alongOf(index));
    moved.insert(QStringLiteral("props"), props);
    const Geometry g = geometryAt(clip, clipTime);
    const QPointF anchor(g.cropL + (1.0 - g.cropL - g.cropR) * (0.5 + g.anchorX), g.cropT + (1.0 - g.cropT - g.cropB) * (0.5 + g.anchorY));
    const QTransform placement = picturePlacement(moved, picture, fit, canvas, clipTime, parents, timeline);
    if (!placement.isInvertible() || !inSight(placement, anchor)) return std::nullopt;
    const QPointF on = placement.map(anchor);
    return QPointF(on.x() / cw, on.y() / ch);
}

} // namespace

QList<std::optional<QPointF>> sampledPathPoints(const QJsonObject &clip, const QSizeF &picture, bool fit, const QSizeF &canvas, double clipTime,
                                                const QTransform &parents, const QJsonObject &timeline, const QList<QPointF> &points) {
    QList<std::optional<QPointF>> places;
    for (qsizetype index = 0; index < points.size(); ++index)
        places.append(anchorAtPoint(clip, picture, fit, canvas, clipTime, parents, timeline, points, int(index)));
    return places;
}

std::optional<QPointF> sampledPathPointFor(const QJsonObject &clip, const QSizeF &picture, bool fit, const QSizeF &canvas, double clipTime,
                                           const QTransform &parents, const QJsonObject &timeline, const QList<QPointF> &points, int index,
                                           const QPointF &seen) {
    if (index < 0 || index >= points.size()) return std::nullopt;
    // Far closer than the document can say a point, which is a ten-thousandth of the frame.
    constexpr double kClose = 1e-7;
    // How far a point is moved to see what that does, and the most a step may take it.
    constexpr double kNudge = 1e-4, kMostStep = 0.5;
    constexpr int kMostSteps = 12;
    QList<QPointF> trying = points;
    const auto landing = [&](const QPointF &point) {
        trying[index] = point;
        return anchorAtPoint(clip, picture, fit, canvas, clipTime, parents, timeline, trying, index);
    };
    QPointF point = points.at(index);
    for (int step = 0; step < kMostSteps; ++step) {
        const auto on = landing(point);
        const auto across = landing(point + QPointF(kNudge, 0.0));
        const auto down = landing(point + QPointF(0.0, kNudge));
        if (!on || !across || !down) return std::nullopt;
        const QPointF off = *on - seen;
        if (std::hypot(off.x(), off.y()) < kClose) return point;
        const double a = (across->x() - on->x()) / kNudge, b = (down->x() - on->x()) / kNudge;
        const double c = (across->y() - on->y()) / kNudge, d = (down->y() - on->y()) / kNudge;
        const double det = a * d - b * c;
        if (std::abs(det) < 1e-12) return std::nullopt;
        QPointF by((d * off.x() - b * off.y()) / det, (a * off.y() - c * off.x()) / det);
        const double length = std::hypot(by.x(), by.y());
        if (length > kMostStep) by *= kMostStep / length;
        point -= by;
    }
    const auto on = landing(point);
    if (!on) return std::nullopt;
    const QPointF off = *on - seen;
    return std::hypot(off.x(), off.y()) < kClose ? std::optional(point) : std::nullopt;
}

QTransform picturePlacement(const QJsonObject &clip, const QSizeF &picture, bool fit, const QSizeF &canvas, double clipTime,
                            const QTransform &parents, const QJsonObject &timeline) {
    const QTransform flat = flatPlacement(clip, picture, fit, canvas, clipTime, parents);
    if (!isInDepth(clip)) return flat;
    // As `Plane::of` lifts it, about its middle at its `z`.
    const Depth depth = depthAt(clip, canvas, clipTime);
    const QPointF middle = flat.map(QPointF(0.5, 0.5));
    const World across = turned({flat.m11(), flat.m12(), 0.0}, depth.pitch, depth.yaw);
    const World below = turned({flat.m21(), flat.m22(), 0.0}, depth.pitch, depth.yaw);
    const World origin = World{middle.x() - canvas.width() / 2.0, middle.y() - canvas.height() / 2.0, depth.z} - across * 0.5 - below * 0.5;
    const QTransform seen = cameraAt(timeline, canvas, clip.value(QStringLiteral("timelineStart")).toDouble() + clipTime).sees(origin, across, below);
    const std::array<QPointF, 4> corners{QPointF(0.0, 0.0), QPointF(1.0, 0.0), QPointF(1.0, 1.0), QPointF(0.0, 1.0)};
    const bool shows = std::any_of(corners.cbegin(), corners.cend(), [&seen](const QPointF &corner) { return inSight(seen, corner); });
    return shows ? seen : unseen();
}

bool inSight(const QTransform &placement, const QPointF &point) {
    return placement.m13() * point.x() + placement.m23() * point.y() + placement.m33() >= kNearest;
}

std::optional<QPointF> pointInSight(const QTransform &placement, const QPointF &seen) {
    bool invertible = false;
    const QTransform back = placement.inverted(&invertible);
    if (!invertible) return std::nullopt;
    // The inverse's third row is one over how far along the camera's way the point is,
    // so nothing or less past the horizon, where QTransform::map would make do with
    // a near limit of its own.
    constexpr double kHorizon = 1e-6;
    if (back.m13() * seen.x() + back.m23() * seen.y() + back.m33() <= kHorizon) return std::nullopt;
    const QPointF point = back.map(seen);
    if (!inSight(placement, point)) return std::nullopt;
    return point;
}

QTransform parentMotion(const QJsonObject &parent, const QSizeF &picture, bool fit, const QSizeF &canvas, double clipTime) {
    const Geometry now = movedGeometryAt(parent, canvas, clipTime);
    Geometry rest;
    rest.cropL = now.cropL;
    rest.cropR = now.cropR;
    rest.cropT = now.cropT;
    rest.cropB = now.cropB;
    rest.anchorX = now.anchorX;
    rest.anchorY = now.anchorY;
    const QTransform before = placed(rest, picture, fit, canvas);
    return before.isInvertible() ? before.inverted() * placed(now, picture, fit, canvas) : QTransform{};
}

QPainterPath maskPath(const QJsonObject &mask) {
    QPainterPath path;
    const auto numbers = [&mask](const char *key) {
        const QJsonArray values = mask.value(QString::fromLatin1(key)).toArray();
        std::array<double, 4> out{};
        for (int index = 0; index < 4 && index < values.size(); ++index) out[index] = values.at(index).toDouble();
        return out;
    };
    if (mask.contains(QStringLiteral("rect"))) {
        const auto [left, top, right, bottom] = numbers("rect");
        path.addRect(QRectF(QPointF(left, top), QPointF(right, bottom)));
    } else if (mask.contains(QStringLiteral("ellipse"))) {
        const auto [x, y, rx, ry] = numbers("ellipse");
        path.addEllipse(QPointF(x, y), rx, ry);
    } else {
        const QJsonArray points = mask.value(QStringLiteral("points")).toArray();
        for (int index = 0; index < points.size(); ++index) {
            const QJsonArray point = points.at(index).toArray();
            const QPointF at(point.at(0).toDouble(), point.at(1).toDouble());
            if (index == 0) path.moveTo(at);
            else path.lineTo(at);
        }
        if (mask.value(QStringLiteral("closed")).toBool(true) && points.size() > 2) path.closeSubpath();
    }
    return path;
}

} // namespace kerf
