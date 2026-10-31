#include "editor_common.h"
#include "editor_theme.h"

#include <QAbstractItemView>
#include <QApplication>
#include <QComboBox>
#include <QCoreApplication>
#include <QDir>
#include <QFileInfo>
#include <QFocusEvent>
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
#include <QTemporaryDir>
#include <QTextEdit>
#include <QToolButton>

#include <algorithm>
#include <array>
#include <cmath>
#include <limits>

namespace kerf {

namespace {

// Where a pop-up row's text starts, past the current choice's check.
constexpr int kPopupTextLeft = 22;

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
            if (directory.exists(QStringLiteral("crates/kerf-mcp/Cargo.toml"))) {
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
    seconds = std::max(0.0, seconds);
    const int minutes = static_cast<int>(seconds) / 60;
    const double remainder = seconds - minutes * 60.0;
    return QStringLiteral("%1:%2.%3")
        .arg(minutes, 2, 10, QLatin1Char('0'))
        .arg(static_cast<int>(remainder), 2, 10, QLatin1Char('0'))
        .arg(static_cast<int>(std::round((remainder - std::floor(remainder)) * 100.0)), 2, 10,
             QLatin1Char('0'));
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

QString timelineRulerLabel(double seconds, double fps, double duration) {
    const QString full = timelineTimecode(seconds, fps);
    return duration >= 3600.0 ? full : full.mid(3);
}

RulerScale rulerScale(double pixelsPerSecond, double fps) {
    const double pps = std::max(pixelsPerSecond, 0.001);
    const double frame = 1.0 / std::max(1.0, fps);
    const double steps[] = {frame,      2 * frame, 5 * frame, 10 * frame, 15 * frame, 1,    2,   5,
                            10,         15,        30,        60,         120,        300, 600, 1800,
                            3600,       7200};
    double major = steps[std::size(steps) - 1];
    for (double step : steps) {
        if (step * pps >= 72.0) {
            major = step;
            break;
        }
    }
    int divisions = 1;
    if (major > frame * 1.01) {
        for (int candidate : {10, 5, 4, 2}) {
            if ((major / candidate) * pps >= 7.0) {
                divisions = candidate;
                break;
            }
        }
    }
    return {major, std::max(1, divisions)};
}

QString clipName(const QJsonObject &clip) {
    if (clip.value(QStringLiteral("name")).isString()) {
        return clip.value(QStringLiteral("name")).toString();
    }
    const QString id = clip.value(QStringLiteral("id")).toString();
    return id.isEmpty() ? QStringLiteral("Untitled clip") : id.left(18);
}

QString sourceName(const QJsonObject &source) {
    const QString path = source.value(QStringLiteral("path")).toString();
    return path.isEmpty() ? QStringLiteral("Untitled media") : QFileInfo(path).fileName();
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

TrackBadge trackBadge(const QJsonObject &project, const QString &trackId) {
    int picture = 0;
    int sound = 0;
    for (const QJsonValue &value : project.value(QStringLiteral("tracks")).toArray()) {
        const QJsonObject track = value.toObject();
        const bool audio = isAudioTrack(track);
        const int number = audio ? ++sound : ++picture;
        if (track.value(QStringLiteral("id")).toString() != trackId) continue;
        QString name = track.value(QStringLiteral("name")).toString().trimmed();
        if (name.isEmpty()) name = (audio ? QStringLiteral("Audio %1") : QStringLiteral("Video %1")).arg(number);
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

QString closeMarkImagePath(const QColor &color) {
    return glyphImagePath(colorKey("close", color), QSize(12, 12), [&color](QPainter &painter) {
        // Through pixel centres, so the strokes are solid even at 1x.
        painter.setPen(QPen(color, 1.4, Qt::SolidLine, Qt::RoundCap));
        painter.drawLine(QPointF(3.5, 3.5), QPointF(8.5, 8.5));
        painter.drawLine(QPointF(8.5, 3.5), QPointF(3.5, 8.5));
    });
}

QString tabsMenuImagePath(const QColor &color) {
    return glyphImagePath(colorKey("tabs-menu", color), QSize(12, 12), [&color](QPainter &painter) {
        painter.setPen(QPen(color, 1.4, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        painter.drawPolyline(QPolygonF{{3.0, 4.75}, {6.0, 7.75}, {9.0, 4.75}});
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
    return {kPopupTextLeft + option.fontMetrics.horizontalAdvance(index.data(Qt::DisplayRole).toString()) + 16, 22};
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
        painter->drawRoundedRect(row, 4, 4);
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

// Fusion, whose push buttons (and check boxes) take focus from Tab alone,
// and whose pop-up buttons' menu windows paint no panel of their own.
class EditorStyle final : public QProxyStyle {
public:
    EditorStyle() : QProxyStyle(QStringLiteral("Fusion")) {}

    int styleHint(StyleHint hint, const QStyleOption *option, const QWidget *widget, QStyleHintReturn *data) const override {
        if (hint == SH_Button_FocusPolicy) return Qt::TabFocus;
        return QProxyStyle::styleHint(hint, option, widget, data);
    }

    void drawPrimitive(PrimitiveElement element, const QStyleOption *option, QPainter *painter, const QWidget *widget) const override {
        // A pop-up button's menu window paints nothing of its own (the style
        // sheet cannot reach it): the list in it, rounded by the style sheet,
        // is the whole menu.
        if (element == PE_PanelMenu && widget && widget->isWindow() && qobject_cast<const QComboBox *>(widget->parentWidget()))
            return;
        QProxyStyle::drawPrimitive(element, option, painter, widget);
    }
};

// The ring around a focused field, or a button or pop-up focused from the
// keyboard: 3 points of the accent at 35% outside its bezel, as the
// launcher rings its fields. It sits in the control's parent, under the
// control, and follows it.
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
        painter.setPen(QPen(editorTheme().focusHalo, kWidth));
        painter.setBrush(Qt::NoBrush);
        // The pen's middle runs half its width outside the bezel.
        const qreal middle = kWidth / 2.0;
        const qreal round = (set ? radius : 5.0) + middle;
        painter.drawRoundedRect(QRectF(rect()).adjusted(middle, middle, -middle, -middle), round, round);
    }

private:
    static constexpr int kWidth = 3;

    void place() {
        if (!m_control) return;
        setGeometry(m_control->geometry().adjusted(-kWidth, -kWidth, kWidth, kWidth));
        stackUnder(m_control);
        setVisible(m_control->isVisible());
    }

    QPointer<QWidget> m_control;
};

// Gives pop-up buttons their rows, pop-up windows their round corners, and
// focused controls their ring, for the whole application.
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
    if (clipTime <= at(0)) return valueOf(0);
    for (int index = 1; index < keys.size(); ++index) {
        if (clipTime > at(index)) continue;
        const double span = at(index) - at(index - 1);
        const double t = span > 0.0 ? (clipTime - at(index - 1)) / span : 1.0;
        return valueOf(index - 1) + (valueOf(index) - valueOf(index - 1)) * t;
    }
    return valueOf(keys.size() - 1);
}

QTransform picturePlacement(const QJsonObject &clip, const QSizeF &picture, bool fit, const QSizeF &canvas,
                            double clipTime) {
    // The compositor's geometry (kerf-compose `Scene::geometry`).
    const QJsonObject props = clip.value(QStringLiteral("props")).toObject();
    const auto prop = [&](const char *name, double fallback) {
        return propertyAt(props, QString::fromLatin1(name), clipTime, fallback);
    };
    const double l = prop("crop_l", 0.0), r = prop("crop_r", 0.0), t = prop("crop_t", 0.0), b = prop("crop_b", 0.0);
    const double sw = std::max(1.0, picture.width()), sh = std::max(1.0, picture.height());
    const double cw = canvas.width(), ch = canvas.height();
    const double width = sw * std::max(1e-6, 1.0 - l - r), height = sh * std::max(1e-6, 1.0 - t - b);
    double fw = width, fh = height;
    if (fit) {
        fw = std::max(1.0, std::min(std::round(ch * width / height), cw));
        fh = std::max(1.0, std::min(std::round(cw * height / width), ch));
    }
    const double scale = prop("scale", 1.0);
    const double kx = scale * prop("scale_x", 1.0), ky = scale * prop("scale_y", 1.0);
    const double ax = prop("anchor_x", 0.0), ay = prop("anchor_y", 0.0);
    QTransform placement;
    placement.translate(cw / 2.0 + prop("x", 0.0) * cw + ax * fw, ch / 2.0 + prop("y", 0.0) * ch + ay * fh);
    placement.rotate(prop("rotation", 0.0));
    placement.scale(fw / width * kx, fh / height * ky);
    placement.translate(-(sw * l + width * (0.5 + ax)), -(sh * t + height * (0.5 + ay)));
    placement.scale(sw, sh);
    return placement;
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
