#include "project_launcher.h"
#include "bridge.h"

#include <QAbstractSpinBox>
#include <QApplication>
#include <QCheckBox>
#include <QComboBox>
#include <QDateTime>
#include <QDir>
#include <QDoubleSpinBox>
#include <QFileDialog>
#include <QFileInfo>
#include <QFocusFrame>
#include <QFontDatabase>
#include <QFormLayout>
#include <QFrame>
#include <QGridLayout>
#include <QGuiApplication>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QJsonDocument>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMessageBox>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPolygonF>
#include <QProgressBar>
#include <QPushButton>
#include <QRegularExpression>
#include <QScrollArea>
#include <QSettings>
#include <QSignalBlocker>
#include <QStackedWidget>
#include <QStandardPaths>
#include <QStyle>
#include <QStyleHints>
#include <QStyledItemDelegate>
#include <QTimer>
#include <QToolButton>
#include <QVBoxLayout>

#include <algorithm>
#include <utility>

namespace kerf {

namespace {

constexpr int kPathRole = Qt::UserRole;
constexpr int kExistsRole = Qt::UserRole + 1;
// A template's canvas size.
constexpr int kSizeRole = Qt::UserRole + 2;
const QString kRecentKey = QStringLiteral("projects/recent");

struct TemplateInfo {
    const char *id;
    const char *label;
    const char *aspect;
    int width;
    int height;
};
constexpr TemplateInfo kTemplates[] = {
    {"standard", "Empty 16:9", "16:9", 1920, 1080},
    {"vertical", "Vertical 9:16", "9:16", 1080, 1920},
    {"square", "Square 1:1", "1:1", 1080, 1080},
};

QJsonObject callJson(char *(*function)(const char *), const QJsonObject &argument) {
    const QByteArray json = QJsonDocument(argument).toJson(QJsonDocument::Compact);
    char *raw = function(json.constData());
    if (!raw) return {{QStringLiteral("error"), QStringLiteral("Kerf did not answer.")}};
    const QJsonDocument document = QJsonDocument::fromJson(QByteArray(raw));
    kerf_bridge_string_free(raw);
    if (!document.isObject()) return {{QStringLiteral("error"), QStringLiteral("Kerf's answer could not be read.")}};
    return document.object();
}

void repolish(QWidget *widget) {
    widget->style()->unpolish(widget);
    widget->style()->polish(widget);
}

QString homeRelative(const QString &path) {
    const QString home = QDir::homePath();
    return path.startsWith(home + QLatin1Char('/')) ? QStringLiteral("~") + path.mid(home.size()) : path;
}

// A location as typed, with `~` for the home folder; empty stays empty.
QString expanded(const QString &text) {
    const QString trimmed = text.trimmed();
    if (trimmed == QStringLiteral("~")) return QDir::homePath();
    if (trimmed.startsWith(QStringLiteral("~/"))) return QDir::cleanPath(QDir::homePath() + trimmed.mid(1));
    return trimmed.isEmpty() ? QString() : QDir::cleanPath(trimmed);
}

// Addresses git clones from and pushes to: URLs, scp-style `user@host:path`,
// and local paths.
bool isGitAddress(const QString &address) {
    static const QRegularExpression url(QStringLiteral(R"(^(https?|ssh|git|file)://\S+$)"));
    static const QRegularExpression scp(QStringLiteral(R"(^[\w.-]+@[\w.-]+:\S+$)"));
    return url.match(address).hasMatch() || scp.match(address).hasMatch() || QDir::isAbsolutePath(address);
}

// The folder scaffold::folder_name makes for a project name.
QString folderName(const QString &name) {
    QString cleaned = name.trimmed();
    for (QChar &c : cleaned)
        if (c == QLatin1Char('/') || c == QLatin1Char('\\') || c == QLatin1Char(':') || c.isNull()) c = QLatin1Char('-');
    while (cleaned.startsWith(QLatin1Char('.'))) cleaned.remove(0, 1);
    while (cleaned.endsWith(QLatin1Char('.'))) cleaned.chop(1);
    return cleaned.trimmed();
}

// A folder a new project or clone cannot go into: a file, or a folder with
// anything in it.
bool occupied(const QString &path) {
    const QFileInfo info(path);
    if (!info.exists()) return false;
    return !info.isDir() ||
           !QDir(path).entryList(QDir::AllEntries | QDir::Hidden | QDir::System | QDir::NoDotAndDotDot).isEmpty();
}

QString defaultLocation() {
    const QString saved = QSettings().value(QStringLiteral("projects/location")).toString();
    if (!saved.isEmpty()) return saved;
    QString movies = QStandardPaths::writableLocation(QStandardPaths::MoviesLocation);
    if (movies.isEmpty()) movies = QDir::homePath();
    return QDir(movies).filePath(QStringLiteral("Kerf"));
}

QString untitledName(const QString &location) {
    QString name = QStringLiteral("Untitled");
    for (int n = 2; occupied(QDir(location).filePath(name)) && n < 1000; ++n)
        name = QStringLiteral("Untitled %1").arg(n);
    return name;
}

bool systemIsDark() {
#if QT_VERSION >= QT_VERSION_CHECK(6, 5, 0)
    const Qt::ColorScheme scheme = QGuiApplication::styleHints()->colorScheme();
    if (scheme != Qt::ColorScheme::Unknown) return scheme == Qt::ColorScheme::Dark;
#endif
    return QGuiApplication::palette().color(QPalette::Window).lightness() < 128;
}

// The accent chosen in System Settings, or macOS's blue where the platform
// has none to give (offscreen test runs among them).
QColor systemAccent(bool dark) {
#if QT_VERSION >= QT_VERSION_CHECK(6, 6, 0)
    const QString platform = QGuiApplication::platformName();
    if (platform != QStringLiteral("offscreen") && platform != QStringLiteral("minimal")) {
        const QColor accent = QGuiApplication::palette().color(QPalette::Accent);
        if (accent.isValid() && accent.alpha() == 255) return accent;
    }
#endif
    return dark ? QColor("#0a84ff") : QColor("#007aff");
}

QPen linePen(const QColor &color, qreal width) {
    return QPen(color, width, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin);
}

// Sidebar symbols: line art on a 16-point grid at `origin`.
void drawSidebarSymbol(QPainter &painter, int page, const QPointF &origin, const QColor &color) {
    painter.save();
    painter.translate(origin);
    painter.setPen(linePen(color, 1.3));
    painter.setBrush(Qt::NoBrush);
    QPainterPath path;
    switch (page) {
    case ProjectLauncher::NewProject:
        path.moveTo(3.5, 2.5); path.lineTo(9.5, 2.5); path.lineTo(12.5, 5.5); path.lineTo(12.5, 13.5);
        path.lineTo(3.5, 13.5); path.closeSubpath();
        path.moveTo(9.5, 2.5); path.lineTo(9.5, 5.5); path.lineTo(12.5, 5.5);
        path.moveTo(8, 7.5); path.lineTo(8, 11.5);
        path.moveTo(6, 9.5); path.lineTo(10, 9.5);
        break;
    case ProjectLauncher::Recent:
        path.addEllipse(QPointF(8, 8), 5.75, 5.75);
        path.moveTo(8, 5); path.lineTo(8, 8); path.lineTo(10.5, 9.5);
        break;
    case ProjectLauncher::FromPreset:
        path.addRoundedRect(QRectF(2.5, 5.5, 11, 8), 1.5, 1.5);
        path.moveTo(4.5, 3.5); path.lineTo(11.5, 3.5);
        path.moveTo(6.5, 7.8); path.lineTo(6.5, 11.2); path.lineTo(9.8, 9.5); path.closeSubpath();
        break;
    case ProjectLauncher::Clone:
        path.moveTo(8, 2.5); path.lineTo(8, 10.5);
        path.moveTo(5, 7.5); path.lineTo(8, 10.5); path.lineTo(11, 7.5);
        path.moveTo(3, 13.5); path.lineTo(13, 13.5);
        break;
    }
    painter.drawPath(path);
    painter.restore();
}

// A project folder in the Recent list.
void drawFolder(QPainter &painter, const QRectF &box, const QColor &color) {
    painter.save();
    painter.setPen(Qt::NoPen);
    painter.setBrush(color.darker(118));
    painter.drawRoundedRect(QRectF(box.left(), box.top(), box.width() * 0.42, 6), 2, 2);
    painter.drawRoundedRect(QRectF(box.left(), box.top() + 2, box.width(), box.height() - 2), 3, 3);
    painter.setBrush(launcherMix(color, Qt::white, 0.22));
    painter.drawRoundedRect(QRectF(box.left(), box.top() + 6, box.width(), box.height() - 6), 3, 3);
    painter.restore();
}

// A preset in the From Preset list: a tinted square with slider marks.
void drawPresetGlyph(QPainter &painter, const QRectF &box, const QColor &color) {
    painter.save();
    QColor fill = color;
    fill.setAlphaF(0.16);
    painter.setPen(Qt::NoPen);
    painter.setBrush(fill);
    painter.drawRoundedRect(box, 5, 5);
    painter.setPen(linePen(color, 1.5));
    const qreal knobs[] = {0.35, 0.68, 0.48};
    for (int i = 0; i < 3; ++i) {
        const qreal y = box.top() + 6 + i * 5;
        painter.drawLine(QPointF(box.left() + 7, y), QPointF(box.right() - 7, y));
        painter.setBrush(color);
        painter.drawEllipse(QPointF(box.left() + 7 + (box.width() - 14) * knobs[i], y), 1.8, 1.8);
    }
    painter.restore();
}

// The Details row's disclosure chevron: › closed, ⌄ open.
QIcon chevronIcon(bool open, const QColor &color) {
    QPixmap pixmap(QSize(12, 12) * 2);
    pixmap.setDevicePixelRatio(2);
    pixmap.fill(Qt::transparent);
    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(linePen(color, 1.5));
    if (open) painter.drawPolyline(QPolygonF{{2.5, 4.5}, {6, 8}, {9.5, 4.5}});
    else painter.drawPolyline(QPolygonF{{4.5, 2.5}, {8, 6}, {4.5, 9.5}});
    painter.end();
    QIcon icon(pixmap);
    icon.addPixmap(pixmap, QIcon::Disabled);
    return icon;
}

// Rows of the sidebar: an accent symbol and a label, on a rounded wash when
// chosen.
class SidebarDelegate final : public QStyledItemDelegate {
public:
    SidebarDelegate(const LauncherTheme *theme, QObject *parent) : QStyledItemDelegate(parent), m_theme(theme) {}

    QSize sizeHint(const QStyleOptionViewItem &, const QModelIndex &) const override { return {0, 30}; }

    void paint(QPainter *painter, const QStyleOptionViewItem &option, const QModelIndex &index) const override {
        const LauncherTheme &t = *m_theme;
        const bool selected = option.state & QStyle::State_Selected;
        const bool enabled = option.state & QStyle::State_Enabled;
        const QRectF row = QRectF(option.rect).adjusted(0, 1, 0, -1);
        painter->save();
        painter->setRenderHint(QPainter::Antialiasing);
        if (selected) {
            painter->setPen(Qt::NoPen);
            painter->setBrush(t.selection);
            painter->drawRoundedRect(row, 6, 6);
        }
        drawSidebarSymbol(*painter, index.row(), QPointF(row.left() + 8, row.center().y() - 8), enabled ? t.accent : t.disabledText);
        QFont font = option.font;
        font.setPixelSize(13);
        font.setWeight(selected ? QFont::DemiBold : QFont::Normal);
        painter->setFont(font);
        painter->setPen(enabled ? t.text : t.disabledText);
        const QRectF label = row.adjusted(32, 0, -8, 0);
        painter->drawText(label, Qt::AlignVCenter | Qt::AlignLeft,
                          QFontMetricsF(font).elidedText(index.data(Qt::DisplayRole).toString(), Qt::ElideRight, label.width()));
        painter->restore();
    }

private:
    const LauncherTheme *m_theme;
};

// Template tiles: a grey tile with the canvas's frame and a + badge, the
// name under it, and the chosen one ringed with its name on an accent pill.
class TileDelegate final : public QStyledItemDelegate {
public:
    static constexpr int kWidth = 150;
    static constexpr int kHeight = 100;
    // Room for the selection ring around the tile.
    static constexpr int kInset = 3;
    static constexpr int kGap = 44;
    static QSize cell() { return {kInset + kWidth + kGap, kInset + kHeight + 56}; }

    TileDelegate(const LauncherTheme *theme, QObject *parent) : QStyledItemDelegate(parent), m_theme(theme) {}

    QSize sizeHint(const QStyleOptionViewItem &, const QModelIndex &) const override { return cell(); }

    void paint(QPainter *painter, const QStyleOptionViewItem &option, const QModelIndex &index) const override {
        const LauncherTheme &t = *m_theme;
        const bool selected = option.state & QStyle::State_Selected;
        painter->save();
        painter->setRenderHint(QPainter::Antialiasing);
        if (!(option.state & QStyle::State_Enabled)) painter->setOpacity(0.5);
        const QRectF tile(option.rect.left() + kInset, option.rect.top() + kInset, kWidth, kHeight);
        if (selected) {
            painter->setPen(QPen(t.accent, 3));
            painter->setBrush(Qt::NoBrush);
            painter->drawRoundedRect(tile.adjusted(-1, -1, 1, 1), 9, 9);
        }
        painter->setPen(Qt::NoPen);
        painter->setBrush(t.tile);
        painter->drawRoundedRect(tile, 8, 8);

        // The canvas at its aspect, with a play mark in it.
        const QSize canvas = index.data(kSizeRole).toSize();
        QSizeF frame(canvas.width(), canvas.height());
        frame.scale(QSizeF(68, 46), Qt::KeepAspectRatio);
        const QPointF center = tile.center() + QPointF(-4, -3);
        const QRectF picture(center.x() - frame.width() / 2, center.y() - frame.height() / 2, frame.width(), frame.height());
        painter->setPen(QPen(t.tileGlyph, 2.4));
        painter->setBrush(Qt::NoBrush);
        painter->drawRoundedRect(picture, 4, 4);
        const QPointF mid = picture.center();
        painter->setPen(Qt::NoPen);
        painter->setBrush(t.tileGlyph);
        painter->drawPolygon(QPolygonF{{mid.x() - 3.5, mid.y() - 5.5}, {mid.x() - 3.5, mid.y() + 5.5}, {mid.x() + 5.5, mid.y()}});
        // The + badge over the frame's lower right corner, cut out of it.
        const QPointF badge = picture.bottomRight() + QPointF(3, 3);
        painter->setBrush(t.tile);
        painter->drawEllipse(badge, 13.5, 13.5);
        painter->setBrush(t.badge);
        painter->drawEllipse(badge, 11, 11);
        painter->setPen(linePen(t.tileGlyph, 2));
        painter->drawLine(badge + QPointF(-5, 0), badge + QPointF(5, 0));
        painter->drawLine(badge + QPointF(0, -5), badge + QPointF(0, 5));

        QFont labelFont = option.font;
        labelFont.setPixelSize(12);
        labelFont.setWeight(QFont::Bold);
        const QFontMetricsF labelMetrics(labelFont);
        const QString label = index.data(Qt::DisplayRole).toString();
        const qreal labelWidth = labelMetrics.horizontalAdvance(label);
        const QRectF pill(tile.center().x() - labelWidth / 2 - 7, tile.bottom() + 9, labelWidth + 14, labelMetrics.height() + 4);
        if (selected) {
            painter->setPen(Qt::NoPen);
            painter->setBrush(t.accent);
            painter->drawRoundedRect(pill, 5, 5);
        }
        painter->setFont(labelFont);
        painter->setPen(selected ? t.onAccent : t.text);
        painter->drawText(pill, Qt::AlignCenter, label);

        QFont sizeFont = option.font;
        sizeFont.setPixelSize(11);
        painter->setFont(sizeFont);
        painter->setPen(t.secondary);
        painter->drawText(QRectF(tile.left() - 8, pill.bottom() + 3, tile.width() + 16, 16), Qt::AlignHCenter | Qt::AlignTop,
                          QStringLiteral("%1 × %2").arg(canvas.width()).arg(canvas.height()));
        painter->restore();
    }

private:
    const LauncherTheme *m_theme;
};

// Two-line rows for Recent and the preset library: a symbol, a bold name,
// and its details in secondary text. The preset library is a grouped list,
// with hairlines between its rows.
class RowDelegate final : public QStyledItemDelegate {
public:
    enum Kind { RecentRow, PresetRow };
    static int rowHeight(Kind kind) { return kind == RecentRow ? 52 : 44; }

    RowDelegate(const LauncherTheme *theme, Kind kind, QObject *parent) : QStyledItemDelegate(parent), m_theme(theme), m_kind(kind) {}

    QSize sizeHint(const QStyleOptionViewItem &, const QModelIndex &) const override { return {0, rowHeight(m_kind)}; }

    void paint(QPainter *painter, const QStyleOptionViewItem &option, const QModelIndex &index) const override {
        const LauncherTheme &t = *m_theme;
        const QRectF rect(option.rect);
        const QString text = index.data(Qt::DisplayRole).toString();
        painter->save();
        painter->setRenderHint(QPainter::Antialiasing);
        QFont detailFont = option.font;
        detailFont.setPixelSize(12);
        if (!index.flags().testFlag(Qt::ItemIsEnabled)) {
            // The note an empty list shows.
            detailFont.setPixelSize(13);
            painter->setFont(detailFont);
            painter->setPen(t.secondary);
            painter->drawText(rect.adjusted(14, 0, -14, 0), Qt::AlignVCenter | Qt::AlignLeft | Qt::TextWordWrap, text);
            painter->restore();
            return;
        }
        const bool selected = option.state & QStyle::State_Selected;
        const bool hovered = option.state & QStyle::State_MouseOver;
        const bool enabled = option.state & QStyle::State_Enabled;
        const QVariant existsData = index.data(kExistsRole);
        const bool exists = !existsData.isValid() || existsData.toBool();
        const bool last = index.row() == index.model()->rowCount() - 1;
        const qreal inset = m_kind == PresetRow ? 4 : 0;
        if (selected || hovered) {
            QColor wash = t.selection;
            if (!selected) wash.setAlphaF(wash.alphaF() / 2);
            painter->setPen(Qt::NoPen);
            painter->setBrush(wash);
            painter->drawRoundedRect(rect.adjusted(inset, inset ? 3 : 1, -inset, inset ? -3 : -1), 6, 6);
        } else if (m_kind == PresetRow && !last) {
            painter->setPen(QPen(t.separator, 1));
            painter->drawLine(QPointF(rect.left() + 52, rect.bottom() - 0.5), QPointF(rect.right() - 12, rect.bottom() - 0.5));
        }
        const QRectF icon(rect.left() + 12, rect.center().y() - 11, 28, 22);
        const QColor tint = exists && enabled ? t.accent : t.disabledText;
        if (m_kind == RecentRow) drawFolder(*painter, icon, tint);
        else drawPresetGlyph(*painter, icon, tint);

        const int split = text.indexOf(QLatin1Char('\n'));
        const QString name = split < 0 ? text : text.left(split);
        const QString detail = split < 0 ? QString() : text.mid(split + 1);
        QFont nameFont = option.font;
        nameFont.setPixelSize(13);
        nameFont.setWeight(QFont::DemiBold);
        const QFontMetricsF nameMetrics(nameFont);
        const QFontMetricsF detailMetrics(detailFont);
        const qreal left = icon.right() + 12;
        const qreal width = rect.right() - 14 - left;
        const qreal height = nameMetrics.height() + 2 + detailMetrics.height();
        const qreal top = rect.center().y() - height / 2;
        painter->setFont(nameFont);
        painter->setPen(exists && enabled ? t.text : t.secondary);
        painter->drawText(QRectF(left, top, width, nameMetrics.height()), Qt::AlignLeft | Qt::AlignVCenter,
                          nameMetrics.elidedText(name, Qt::ElideRight, width));
        painter->setFont(detailFont);
        painter->setPen(t.secondary);
        painter->drawText(QRectF(left, top + nameMetrics.height() + 2, width, detailMetrics.height()), Qt::AlignLeft | Qt::AlignVCenter,
                          detailMetrics.elidedText(detail, Qt::ElideMiddle, width));
        painter->restore();
    }

private:
    const LauncherTheme *m_theme;
    Kind m_kind;
};

// A macOS switch in front of its label. It stays a QCheckBox to everything
// that reads or sets it.
class LauncherSwitch final : public QCheckBox {
public:
    LauncherSwitch(const QString &text, const LauncherTheme *theme) : QCheckBox(text), m_theme(theme) {
        // Clicks do not take focus; Tab does, and shows a ring.
        setFocusPolicy(Qt::TabFocus);
    }

    QSize sizeHint() const override {
        const QFontMetrics metrics(font());
        return {kTrackWidth + kGap + metrics.horizontalAdvance(text()) + 4, std::max(24, metrics.height() + 6)};
    }
    QSize minimumSizeHint() const override { return sizeHint(); }

protected:
    bool hitButton(const QPoint &pos) const override { return rect().contains(pos); }

    void paintEvent(QPaintEvent *) override {
        const LauncherTheme &t = *m_theme;
        QPainter painter(this);
        painter.setRenderHint(QPainter::Antialiasing);
        const bool on = isChecked();
        const bool enabled = isEnabled();
        const QRectF track(2, (height() - kTrackHeight) / 2.0, kTrackWidth, kTrackHeight);
        if (hasFocus()) {
            painter.setPen(QPen(t.focusRing, 3));
            painter.setBrush(Qt::NoBrush);
            painter.drawRoundedRect(track.adjusted(-1.5, -1.5, 1.5, 1.5), kTrackHeight / 2.0 + 1.5, kTrackHeight / 2.0 + 1.5);
        }
        painter.setOpacity(enabled ? 1.0 : 0.45);
        painter.setPen(Qt::NoPen);
        painter.setBrush(on ? t.accent : t.switchOff);
        painter.drawRoundedRect(track, kTrackHeight / 2.0, kTrackHeight / 2.0);
        const qreal knob = kTrackHeight - 4;
        const QRectF knobRect(on ? track.right() - 2 - knob : track.left() + 2, track.top() + 2, knob, knob);
        painter.setBrush(QColor(0, 0, 0, t.dark ? 80 : 45));
        painter.drawEllipse(knobRect.translated(0, 0.7));
        painter.setBrush(t.knob);
        painter.drawEllipse(knobRect);
        painter.setOpacity(1.0);
        painter.setFont(font());
        painter.setPen(enabled ? t.text : t.disabledText);
        const qreal textLeft = track.right() + kGap;
        painter.drawText(QRectF(textLeft, 0, width() - textLeft, height()), Qt::AlignVCenter | Qt::AlignLeft, text());
    }

private:
    static constexpr int kTrackWidth = 28;
    static constexpr int kTrackHeight = 16;
    static constexpr int kGap = 8;
    const LauncherTheme *m_theme;
};

// A pop-up button: a field with macOS's up and down chevrons.
class LauncherPopup final : public QComboBox {
public:
    explicit LauncherPopup(const LauncherTheme *theme) : m_theme(theme) {}

protected:
    void paintEvent(QPaintEvent *event) override {
        QComboBox::paintEvent(event);
        QPainter painter(this);
        painter.setRenderHint(QPainter::Antialiasing);
        painter.setPen(linePen(isEnabled() ? m_theme->secondary : m_theme->disabledText, 1.5));
        const QPointF mid(width() - 15, height() / 2.0);
        painter.drawPolyline(QPolygonF{mid + QPointF(-3, -1.5), mid + QPointF(0, -4.5), mid + QPointF(3, -1.5)});
        painter.drawPolyline(QPolygonF{mid + QPointF(-3, 1.5), mid + QPointF(0, 4.5), mid + QPointF(3, 1.5)});
    }

private:
    const LauncherTheme *m_theme;
};

// One line of text that gives up its middle rather than widen the window.
class ElidedLabel final : public QLabel {
public:
    using QLabel::QLabel;
    QSize sizeHint() const override { return {0, QLabel::sizeHint().height()}; }
    QSize minimumSizeHint() const override { return {0, QLabel::minimumSizeHint().height()}; }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter painter(this);
        painter.setFont(font());
        painter.setPen(palette().color(QPalette::WindowText));
        const QRect area = contentsRect();
        painter.drawText(area, int(alignment()), fontMetrics().elidedText(text(), Qt::ElideMiddle, area.width()));
    }
};

// The soft accent ring macOS draws around the field being typed in. It sits
// under the field, so only the part outside its border shows.
class FocusRing final : public QFocusFrame {
public:
    FocusRing(const LauncherTheme *theme, QWidget *parent) : QFocusFrame(parent), m_theme(theme) {}

protected:
    void paintEvent(QPaintEvent *) override {
        if (!widget()) return;
        QPainter painter(this);
        painter.setRenderHint(QPainter::Antialiasing);
        painter.setPen(QPen(m_theme->focusRing, 3));
        painter.setBrush(Qt::NoBrush);
        painter.drawRoundedRect(QRectF(rect()).adjusted(1.5, 1.5, -1.5, -1.5), 7.5, 7.5);
    }

private:
    const LauncherTheme *m_theme;
};

// Runs `click` when the watched widget is clicked.
class ClickFilter final : public QObject {
public:
    ClickFilter(std::function<void()> click, QObject *parent) : QObject(parent), m_click(std::move(click)) {}

protected:
    bool eventFilter(QObject *watched, QEvent *event) override {
        if (event->type() == QEvent::MouseButtonRelease && static_cast<QMouseEvent *>(event)->button() == Qt::LeftButton) {
            m_click();
            return true;
        }
        return QObject::eventFilter(watched, event);
    }

private:
    std::function<void()> m_click;
};

QPushButton *chooseButton(const QString &objectName) {
    auto *button = new QPushButton(QStringLiteral("Choose…"));
    button->setObjectName(objectName);
    return button;
}

// A right-aligned form label on the first line of its field: QFormLayout
// gives labels of tall rows extra height, so it keeps to the top.
QLabel *formLabel(const QString &text) {
    auto *label = new QLabel(text);
    label->setObjectName(QStringLiteral("launcherFormLabel"));
    label->setAlignment(Qt::AlignRight | Qt::AlignTop);
    return label;
}

// Forms keep kRingRoom around their fields for the focus ring, which is
// drawn in the field's parent and would be clipped at its edges.
constexpr int kRingRoom = 3;

QFormLayout *launcherForm(QWidget *parent) {
    auto *form = new QFormLayout(parent);
    form->setContentsMargins(kRingRoom, kRingRoom, kRingRoom, kRingRoom);
    form->setHorizontalSpacing(12);
    form->setVerticalSpacing(8);
    form->setLabelAlignment(Qt::AlignRight | Qt::AlignTop);
    form->setFormAlignment(Qt::AlignLeft | Qt::AlignTop);
    form->setFieldGrowthPolicy(QFormLayout::AllNonFixedFieldsGrow);
    form->setRowWrapPolicy(QFormLayout::DontWrapRows);
    return form;
}

QFrame *groupSeparator() {
    auto *line = new QFrame;
    line->setObjectName(QStringLiteral("launcherGroupSeparator"));
    line->setFixedHeight(1);
    return line;
}

} // namespace

ProjectLauncher::ProjectLauncher(const QString &kerfdBinary, QWidget *parent) : QDialog(parent), m_kerfd(kerfdBinary) {
    setObjectName(QStringLiteral("projectLauncher"));
    setWindowTitle(QStringLiteral("Choose a Project"));
    setAttribute(Qt::WA_StyledBackground);
    // The system's interface font, not the editor's.
    QFont interfaceFont = QFontDatabase::systemFont(QFontDatabase::GeneralFont);
    interfaceFont.setPixelSize(13);
    setFont(interfaceFont);
    resize(1040, 650);
    setMinimumSize(860, 560);
    const QByteArray forced = qgetenv("KERF_COLOR_SCHEME").trimmed().toLower();
    if (forced == "light") m_scheme = ColorScheme::Light;
    else if (forced == "dark") m_scheme = ColorScheme::Dark;
    m_inspectProject = [](const QJsonObject &request) { return callJson(kerf_project_inspect, request); };
    m_inspectPreset = [](const QJsonObject &request) { return callJson(kerf_preset_inspect, request); };

    auto *root = new QHBoxLayout(this);
    root->setContentsMargins(0, 0, 0, 0);
    root->setSpacing(0);

    m_sidebar = new QListWidget;
    m_sidebar->setObjectName(QStringLiteral("launcherSidebar"));
    m_sidebar->setFixedWidth(210);
    m_sidebar->setFrameShape(QFrame::NoFrame);
    m_sidebar->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_sidebar->setItemDelegate(new SidebarDelegate(&m_theme, m_sidebar));
    for (const QString &label : {QStringLiteral("New Project"), QStringLiteral("Recent"), QStringLiteral("From Preset"),
                                 QStringLiteral("Clone from GitHub")})
        m_sidebar->addItem(label);
    root->addWidget(m_sidebar);

    auto *content = new QWidget;
    content->setObjectName(QStringLiteral("launcherContent"));
    content->setAttribute(Qt::WA_StyledBackground);
    auto *column = new QVBoxLayout(content);
    column->setContentsMargins(0, 0, 0, 0);
    column->setSpacing(0);
    auto *pagesArea = new QWidget;
    pagesArea->setObjectName(QStringLiteral("launcherPagesArea"));
    auto *pagesLayout = new QVBoxLayout(pagesArea);
    pagesLayout->setContentsMargins(28, 20, 28, 16);
    pagesLayout->setSpacing(3);
    m_heading = new QLabel;
    m_heading->setObjectName(QStringLiteral("launcherHeading"));
    pagesLayout->addWidget(m_heading);
    m_subheading = new QLabel;
    m_subheading->setObjectName(QStringLiteral("launcherHint"));
    m_subheading->setWordWrap(true);
    pagesLayout->addWidget(m_subheading);
    pagesLayout->addSpacing(16);
    m_pages = new QStackedWidget;
    m_pages->setObjectName(QStringLiteral("launcherPages"));
    m_pages->addWidget(buildTemplates());
    m_pages->addWidget(buildRecent());
    m_pages->addWidget(buildPresetPage());
    m_pages->addWidget(buildClone());
    pagesLayout->addWidget(m_pages, 1);
    column->addWidget(pagesArea, 1);
    column->addWidget(buildDetails());
    column->addWidget(buildFooter());
    root->addWidget(content, 1);

    connect(m_sidebar, &QListWidget::currentRowChanged, this, [this](int row) {
        if (row >= 0) showPage(static_cast<Page>(row));
    });
    connect(static_cast<QApplication *>(QCoreApplication::instance()), &QApplication::focusChanged, this,
            [this](QWidget *, QWidget *now) { trackFocus(now); });
#if QT_VERSION >= QT_VERSION_CHECK(6, 5, 0)
    connect(QGuiApplication::styleHints(), &QStyleHints::colorSchemeChanged, this, [this] { applyTheme(); });
#endif
    applyTheme(true);
    reloadRecent();
    reloadPresetChoices();
    const int saved = QSettings().value(QStringLiteral("launcher/page"), int(NewProject)).toInt();
    showPage(saved >= NewProject && saved <= Clone ? static_cast<Page>(saved) : NewProject);

    // What can share a project: git, Git LFS, and a signed-in GitHub CLI.
    runTask(QJsonObject{{QStringLiteral("type"), QStringLiteral("tools")}}, [this](const QJsonObject &status) {
        const QString state = status.value(QStringLiteral("status")).toString();
        if (state == QStringLiteral("done")) {
            setTools(status);
        } else if (state != QStringLiteral("running")) {
            // Sharing still works when git is there; the task says if not.
            m_gitHint->setText(QStringLiteral("Kerf could not look for Git and the GitHub CLI: %1")
                                   .arg(status.value(QStringLiteral("error")).toString()));
        }
    });
}

ProjectLauncher::~ProjectLauncher() {
#ifndef KERF_QT_TESTING
    for (void *task : std::as_const(m_tasks)) kerf_task_free(task);
#endif
}

void ProjectLauncher::setColorScheme(ColorScheme scheme) {
    m_scheme = scheme;
    applyTheme();
}

void ProjectLauncher::applyTheme(bool force) {
    if (m_theming) return;
    const bool dark = m_scheme == ColorScheme::Dark || (m_scheme == ColorScheme::System && systemIsDark());
    const QColor accent = systemAccent(dark);
    if (!force && m_themed && dark == m_theme.dark && accent == m_theme.accent) return;
    m_theming = true;
    m_theme = launcherTheme(dark, accent);
    m_themed = true;
    setStyleSheet(launcherStyleSheet(m_theme));
    m_detailsToggle->setIcon(chevronIcon(m_detailsToggle->isChecked(), m_theme.secondary));
    for (QListWidget *list : {m_sidebar, m_templates, m_recent, m_presetChoices}) list->viewport()->update();
    update();
    m_theming = false;
}

void ProjectLauncher::changeEvent(QEvent *event) {
    QDialog::changeEvent(event);
    // Where Qt has no colour scheme to report, a new system palette is how
    // a change of appearance or accent arrives.
    if (event->type() == QEvent::ApplicationPaletteChange || event->type() == QEvent::PaletteChange) applyTheme();
}

void ProjectLauncher::trackFocus(QWidget *now) {
    QWidget *field = nullptr;
    if (now && isAncestorOf(now)) {
        if (qobject_cast<QLineEdit *>(now) && qobject_cast<QAbstractSpinBox *>(now->parentWidget())) field = now->parentWidget();
        else if (qobject_cast<QLineEdit *>(now) || qobject_cast<QComboBox *>(now) || qobject_cast<QAbstractSpinBox *>(now)) field = now;
    }
    if (!field) {
        if (m_focusRing) m_focusRing->setWidget(nullptr);
        return;
    }
    // The ring lives beside its field, so it goes when the field's parent does.
    if (!m_focusRing) m_focusRing = new FocusRing(&m_theme, this);
    m_focusRing->setWidget(field);
}

QWidget *ProjectLauncher::buildTemplates() {
    m_templates = new QListWidget;
    m_templates->setObjectName(QStringLiteral("templateTiles"));
    m_templates->setViewMode(QListView::IconMode);
    m_templates->setMovement(QListView::Static);
    m_templates->setResizeMode(QListView::Adjust);
    m_templates->setSpacing(0);
    m_templates->setUniformItemSizes(true);
    m_templates->setGridSize(TileDelegate::cell());
    m_templates->setFrameShape(QFrame::NoFrame);
    m_templates->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_templates->setItemDelegate(new TileDelegate(&m_theme, m_templates));
    for (const TemplateInfo &entry : kTemplates) {
        auto *item = new QListWidgetItem(QString::fromLatin1(entry.label));
        item->setData(kPathRole, QString::fromLatin1(entry.id));
        item->setData(kSizeRole, QSize(entry.width, entry.height));
        m_templates->addItem(item);
    }
    const QString saved = QSettings().value(QStringLiteral("projects/template"), QStringLiteral("standard")).toString();
    for (int row = 0; row < m_templates->count(); ++row)
        if (m_templates->item(row)->data(kPathRole).toString() == saved) m_templates->setCurrentRow(row);
    if (!m_templates->currentItem()) m_templates->setCurrentRow(0);
    connect(m_templates, &QListWidget::currentItemChanged, this, [this] {
        updateDetailsSummary();
        updateChoose();
    });
    connect(m_templates, &QListWidget::itemActivated, this, [this] { choose(); });
    return m_templates;
}

QWidget *ProjectLauncher::buildRecent() {
    m_recent = new QListWidget;
    m_recent->setObjectName(QStringLiteral("recentProjects"));
    m_recent->setFrameShape(QFrame::NoFrame);
    m_recent->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_recent->setMouseTracking(true);
    m_recent->viewport()->setAttribute(Qt::WA_Hover);
    m_recent->setItemDelegate(new RowDelegate(&m_theme, RowDelegate::RecentRow, m_recent));
    connect(m_recent, &QListWidget::currentItemChanged, this, [this] { updateChoose(); });
    connect(m_recent, &QListWidget::itemActivated, this, [this] { choose(); });
    return m_recent;
}

QWidget *ProjectLauncher::buildPresetPage() {
    auto *page = new QWidget;
    auto *layout = new QVBoxLayout(page);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(8);
    auto *pickRow = new QHBoxLayout;
    auto *libraryLabel = new QLabel(QStringLiteral("Presets in your projects"));
    libraryLabel->setObjectName(QStringLiteral("launcherEyebrow"));
    pickRow->addWidget(libraryLabel);
    pickRow->addStretch(1);
    auto *pickFile = new QPushButton(QStringLiteral("Choose a Preset File…"));
    pickFile->setObjectName(QStringLiteral("presetChooseFile"));
    connect(pickFile, &QPushButton::clicked, this, [this] {
        const QString path = QFileDialog::getOpenFileName(this, QStringLiteral("Choose a preset"), defaultLocation(),
                                                          QStringLiteral("Kerf editing preset (*.kerfpreset)"));
        if (!path.isEmpty()) loadPreset(path);
    });
    pickRow->addWidget(pickFile);
    layout->addLayout(pickRow);
    m_presetChoices = new QListWidget;
    m_presetChoices->setObjectName(QStringLiteral("presetChoices"));
    m_presetChoices->setFrameShape(QFrame::NoFrame);
    m_presetChoices->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_presetChoices->setMouseTracking(true);
    m_presetChoices->viewport()->setAttribute(Qt::WA_Hover);
    m_presetChoices->setItemDelegate(new RowDelegate(&m_theme, RowDelegate::PresetRow, m_presetChoices));
    connect(m_presetChoices, &QListWidget::currentItemChanged, this, [this](QListWidgetItem *item) {
        if (item && item->flags().testFlag(Qt::ItemIsEnabled)) loadPreset(item->data(kPathRole).toString());
    });
    layout->addWidget(m_presetChoices);
    layout->addSpacing(10);
    m_presetTitle = new QLabel;
    m_presetTitle->setObjectName(QStringLiteral("launcherEyebrow"));
    m_presetTitle->setWordWrap(true);
    layout->addWidget(m_presetTitle);
    auto *scroll = new QScrollArea;
    scroll->setObjectName(QStringLiteral("presetSlotsScroll"));
    scroll->setWidgetResizable(true);
    scroll->setFrameShape(QFrame::NoFrame);
    scroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_presetSlots = new QWidget;
    m_presetSlots->setObjectName(QStringLiteral("presetSlots"));
    m_presetSlotsLayout = new QVBoxLayout(m_presetSlots);
    m_presetSlotsLayout->setContentsMargins(0, 0, 0, 0);
    m_presetSlotsLayout->setSpacing(12);
    m_presetSlotsLayout->addStretch(1);
    scroll->setWidget(m_presetSlots);
    layout->addWidget(scroll, 1);
    return page;
}

QWidget *ProjectLauncher::buildClone() {
    auto *page = new QWidget;
    auto *pageLayout = new QHBoxLayout(page);
    pageLayout->setContentsMargins(0, 4 - kRingRoom, 0, 0);
    auto *formBox = new QWidget;
    formBox->setMaximumWidth(620);
    auto *form = launcherForm(formBox);
    m_cloneUrl = new QLineEdit;
    m_cloneUrl->setObjectName(QStringLiteral("cloneUrl"));
    m_cloneUrl->setPlaceholderText(QStringLiteral("https://github.com/you/film.git"));
    form->addRow(formLabel(QStringLiteral("Repository")), m_cloneUrl);
    auto *locationRow = new QHBoxLayout;
    locationRow->setSpacing(8);
    m_cloneLocation = new QLineEdit(defaultLocation());
    m_cloneLocation->setObjectName(QStringLiteral("cloneLocation"));
    locationRow->addWidget(m_cloneLocation, 1);
    auto *pick = chooseButton(QStringLiteral("cloneLocationChoose"));
    connect(pick, &QPushButton::clicked, this, [this] {
        const QString path = QFileDialog::getExistingDirectory(this, QStringLiteral("Clone into"), m_cloneLocation->text());
        if (!path.isEmpty()) m_cloneLocation->setText(path);
    });
    locationRow->addWidget(pick);
    form->addRow(formLabel(QStringLiteral("Location")), locationRow);
    auto *notes = new QVBoxLayout;
    notes->setSpacing(10);
    m_cloneDestination = new QLabel;
    m_cloneDestination->setObjectName(QStringLiteral("launcherHint"));
    notes->addWidget(m_cloneDestination);
    auto *note = new QLabel(QStringLiteral("Kerf configures the repository so project.kerf merges by clip and track, "
                                           "and downloads Git LFS media when it is installed."));
    note->setObjectName(QStringLiteral("launcherHint"));
    note->setWordWrap(true);
    notes->addWidget(note);
    form->addRow(QString(), notes);
    pageLayout->addWidget(formBox, 100, Qt::AlignTop);
    pageLayout->addStretch(1);
    const auto changed = [this] {
        const QString folder = cloneFolder();
        m_cloneDestination->setText(folder.isEmpty() ? QString()
                                                     : QStringLiteral("Clones into %1").arg(homeRelative(folder)));
        updateChoose();
    };
    connect(m_cloneUrl, &QLineEdit::textChanged, this, changed);
    connect(m_cloneLocation, &QLineEdit::textChanged, this, changed);
    return page;
}

QWidget *ProjectLauncher::buildDetails() {
    m_details = new QWidget;
    m_details->setObjectName(QStringLiteral("launcherDetails"));
    auto *layout = new QVBoxLayout(m_details);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);

    // A full-width row as in macOS's Choose a Project windows: the
    // disclosure, then what Choose will make.
    m_detailsRow = new QWidget;
    m_detailsRow->setObjectName(QStringLiteral("launcherDetailsRow"));
    m_detailsRow->setAttribute(Qt::WA_StyledBackground);
    m_detailsRow->setFixedHeight(38);
    auto *rowLayout = new QHBoxLayout(m_detailsRow);
    rowLayout->setContentsMargins(18, 0, 20, 0);
    rowLayout->setSpacing(8);
    m_detailsToggle = new QToolButton;
    m_detailsToggle->setObjectName(QStringLiteral("launcherDetailsToggle"));
    m_detailsToggle->setCheckable(true);
    m_detailsToggle->setText(QStringLiteral("Details"));
    m_detailsToggle->setToolButtonStyle(Qt::ToolButtonTextBesideIcon);
    m_detailsToggle->setIconSize(QSize(12, 12));
    m_detailsToggle->setFixedWidth(72);
    rowLayout->addWidget(m_detailsToggle);
    m_detailsSummary = new ElidedLabel;
    m_detailsSummary->setObjectName(QStringLiteral("launcherDetailsSummary"));
    m_detailsSummary->setAlignment(Qt::AlignCenter);
    rowLayout->addWidget(m_detailsSummary, 1);
    // Balances the toggle, so the summary sits in the middle.
    rowLayout->addSpacing(72);
    auto *click = new ClickFilter([this] { m_detailsToggle->toggle(); }, this);
    m_detailsRow->installEventFilter(click);
    m_detailsSummary->installEventFilter(click);
    layout->addWidget(m_detailsRow);

    m_detailsBody = new QWidget;
    m_detailsBody->setObjectName(QStringLiteral("launcherDetailsBody"));
    auto *bodyLayout = new QHBoxLayout(m_detailsBody);
    bodyLayout->setContentsMargins(20 - kRingRoom, 6 - kRingRoom, 20 - kRingRoom, 18 - kRingRoom);
    auto *formBox = new QWidget;
    formBox->setMaximumWidth(620);
    auto *form = launcherForm(formBox);
    const QString location = defaultLocation();
    m_name = new QLineEdit(untitledName(location));
    m_name->setObjectName(QStringLiteral("launcherName"));
    form->addRow(formLabel(QStringLiteral("Name")), m_name);
    auto *locationRow = new QHBoxLayout;
    locationRow->setSpacing(8);
    m_location = new QLineEdit(location);
    m_location->setObjectName(QStringLiteral("launcherLocation"));
    locationRow->addWidget(m_location, 1);
    auto *pick = chooseButton(QStringLiteral("launcherLocationChoose"));
    connect(pick, &QPushButton::clicked, this, [this] {
        const QString path = QFileDialog::getExistingDirectory(this, QStringLiteral("Save projects in"), m_location->text());
        if (!path.isEmpty()) m_location->setText(path);
    });
    locationRow->addWidget(pick);
    form->addRow(formLabel(QStringLiteral("Location")), locationRow);
    m_rate = new LauncherPopup(&m_theme);
    m_rate->setObjectName(QStringLiteral("launcherRate"));
    m_rate->setMinimumWidth(130);
    for (const char *rate : {"23.976", "24", "25", "29.97", "30", "60"})
        m_rate->addItem(QStringLiteral("%1 fps").arg(QString::fromLatin1(rate)), QString::fromLatin1(rate));
    m_rate->setCurrentIndex(std::max(0, m_rate->findData(QSettings().value(QStringLiteral("projects/rate"), QStringLiteral("30")).toString())));
    auto *rateRow = new QHBoxLayout;
    rateRow->addWidget(m_rate);
    rateRow->addStretch(1);
    form->addRow(formLabel(QStringLiteral("Frame Rate")), rateRow);
    auto *sharing = new QVBoxLayout;
    sharing->setSpacing(4);
    m_git = new LauncherSwitch(QStringLiteral("Share with Git"), &m_theme);
    m_git->setObjectName(QStringLiteral("launcherGit"));
    m_git->setToolTip(QStringLiteral("Make the project a Git repository, so collaborators pull, merge, and push its edits."));
    sharing->addWidget(m_git);
    m_github = new LauncherSwitch(QStringLiteral("Create a private GitHub repository"), &m_theme);
    m_github->setObjectName(QStringLiteral("launcherGitHub"));
    m_github->setEnabled(false);
    sharing->addWidget(m_github);
    sharing->addSpacing(4);
    m_remote = new QLineEdit;
    m_remote->setObjectName(QStringLiteral("launcherRemote"));
    m_remote->setPlaceholderText(QStringLiteral("or push to a remote: git@github.com:you/film.git"));
    sharing->addWidget(m_remote);
    m_gitHint = new QLabel(QStringLiteral("Looking for Git and the GitHub CLI…"));
    m_gitHint->setObjectName(QStringLiteral("launcherHint"));
    m_gitHint->setWordWrap(true);
    sharing->addWidget(m_gitHint);
    form->addRow(formLabel(QStringLiteral("Sharing")), sharing);
    m_destination = new QLabel;
    m_destination->setObjectName(QStringLiteral("launcherHint"));
    form->addRow(QString(), m_destination);
    bodyLayout->addStretch(1);
    bodyLayout->addWidget(formBox, 100);
    bodyLayout->addStretch(1);
    layout->addWidget(m_detailsBody);

    // Closed until opened once, as in the window this follows.
    const bool open = QSettings().value(QStringLiteral("launcher/details"), false).toBool();
    m_detailsToggle->setChecked(open);
    setDetailsOpen(open);
    connect(m_detailsToggle, &QToolButton::toggled, this, [this](bool show) {
        setDetailsOpen(show);
        QSettings().setValue(QStringLiteral("launcher/details"), show);
    });
    const auto changed = [this] {
        updateDetailsSummary();
        updateChoose();
    };
    connect(m_name, &QLineEdit::textChanged, this, changed);
    connect(m_location, &QLineEdit::textChanged, this, changed);
    connect(m_rate, &QComboBox::currentIndexChanged, this, changed);
    connect(m_git, &QCheckBox::toggled, this, [this, changed](bool on) {
        if (!on) {
            m_github->setChecked(false);
            m_remote->clear();
        }
        m_remote->setEnabled(!m_github->isChecked());
        changed();
    });
    connect(m_github, &QCheckBox::toggled, this, [this, changed](bool on) {
        if (on) m_git->setChecked(true);
        m_remote->setEnabled(!on);
        changed();
    });
    connect(m_remote, &QLineEdit::textChanged, this, [this, changed](const QString &text) {
        if (!text.trimmed().isEmpty()) m_git->setChecked(true);
        changed();
    });
    updateDetailsSummary();
    return m_details;
}

void ProjectLauncher::setDetailsOpen(bool open) {
    m_detailsBody->setVisible(open);
    m_detailsToggle->setIcon(chevronIcon(open, m_theme.secondary));
}

QWidget *ProjectLauncher::buildFooter() {
    auto *footer = new QFrame;
    footer->setObjectName(QStringLiteral("launcherFooter"));
    auto *layout = new QHBoxLayout(footer);
    layout->setContentsMargins(20, 12, 20, 12);
    layout->setSpacing(10);
    m_openExisting = new QPushButton(QStringLiteral("Open an Existing Project…"));
    m_openExisting->setObjectName(QStringLiteral("launcherOpenExisting"));
    connect(m_openExisting, &QPushButton::clicked, this, &ProjectLauncher::openExisting);
    layout->addWidget(m_openExisting);
    layout->addSpacing(6);
    m_progress = new QProgressBar;
    m_progress->setObjectName(QStringLiteral("launcherProgress"));
    m_progress->setRange(0, 0);
    m_progress->setTextVisible(false);
    m_progress->setFixedWidth(90);
    m_progress->hide();
    layout->addWidget(m_progress);
    m_problem = new QLabel;
    m_problem->setObjectName(QStringLiteral("launcherProblem"));
    m_problem->setWordWrap(true);
    m_problem->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
    layout->addWidget(m_problem, 1);
    layout->addSpacing(6);
    m_cancel = new QPushButton(QStringLiteral("Cancel"));
    m_cancel->setObjectName(QStringLiteral("launcherCancel"));
    m_cancel->setMinimumWidth(84);
    connect(m_cancel, &QPushButton::clicked, this, &QDialog::reject);
    layout->addWidget(m_cancel);
    m_choose = new QPushButton(QStringLiteral("Choose"));
    m_choose->setObjectName(QStringLiteral("launcherChoose"));
    m_choose->setDefault(true);
    m_choose->setMinimumWidth(84);
    connect(m_choose, &QPushButton::clicked, this, &ProjectLauncher::choose);
    layout->addWidget(m_choose);
    return footer;
}

void ProjectLauncher::showPage(Page page) {
    {
        const QSignalBlocker blocker(m_sidebar);
        m_sidebar->setCurrentRow(page);
    }
    m_pages->setCurrentIndex(page);
    static const char *const headings[][2] = {
        {"Choose a template", "A project is a folder: project.kerf, a media/ folder, and a presets/ library."},
        {"Open a recent project", "Double-click a project to open it."},
        {"Start from a shared preset",
         "Fill the preset's media slots with your own files. Its cuts, effects, and timing stay as they are."},
        {"Clone a shared project", "Copy a project from GitHub or any Git remote, with its history."},
    };
    m_heading->setText(QString::fromUtf8(headings[page][0]));
    m_subheading->setText(QString::fromUtf8(headings[page][1]));
    m_details->setVisible(page == NewProject || page == FromPreset);
    QSettings().setValue(QStringLiteral("launcher/page"), int(page));
    updateDetailsSummary();
    updateChoose();
}

QStringList ProjectLauncher::recentProjects() {
    QStringList recent;
    for (const QString &path : QSettings().value(kRecentKey).toStringList())
        if (!path.isEmpty() && !recent.contains(path)) recent.append(path);
    return recent.mid(0, kRecentLimit);
}

void ProjectLauncher::rememberProject(const QString &path) {
    if (path.isEmpty()) return;
    const QString clean = QDir::cleanPath(QFileInfo(path).absoluteFilePath());
    QStringList recent = recentProjects();
    recent.removeAll(clean);
    recent.prepend(clean);
    QSettings().setValue(kRecentKey, recent.mid(0, kRecentLimit));
}

void ProjectLauncher::clearRecentProjects() {
    QSettings().remove(kRecentKey);
}

void ProjectLauncher::reloadRecent() {
    m_recent->clear();
    for (const QString &path : recentProjects()) {
        const QJsonObject info = m_inspectProject(QJsonObject{{QStringLiteral("path"), path}});
        const bool exists = info.contains(QStringLiteral("exists")) ? info.value(QStringLiteral("exists")).toBool()
                                                                     : QFileInfo(path).isDir();
        QString name = info.value(QStringLiteral("name")).toString();
        if (name.isEmpty()) name = QFileInfo(path).fileName();
        QStringList details{homeRelative(path)};
        if (!exists) {
            details << QStringLiteral("Not found");
        } else {
            const QString branch = info.value(QStringLiteral("branch")).toString();
            if (!branch.isEmpty()) details << QStringLiteral("⎇ %1").arg(branch);
            const QDateTime modified = QDateTime::fromString(info.value(QStringLiteral("modified")).toString(), Qt::ISODate);
            if (modified.isValid()) details << QStringLiteral("Edited %1").arg(modified.toLocalTime().toString(QStringLiteral("d MMM yyyy, HH:mm")));
            const int presets = info.value(QStringLiteral("presets")).toInt();
            if (presets > 0) details << (presets == 1 ? QStringLiteral("1 preset") : QStringLiteral("%1 presets").arg(presets));
        }
        auto *item = new QListWidgetItem(QStringLiteral("%1\n%2").arg(name, details.join(QStringLiteral("  ·  "))));
        item->setData(kPathRole, path);
        item->setData(kExistsRole, exists);
        item->setToolTip(path);
        m_recent->addItem(item);
    }
    if (m_recent->count() == 0) {
        auto *empty = new QListWidgetItem(QStringLiteral("No recent projects yet. Projects you open appear here."));
        empty->setFlags(Qt::NoItemFlags);
        m_recent->addItem(empty);
    } else {
        m_recent->setCurrentRow(0);
    }
}

void ProjectLauncher::reloadPresetChoices() {
    m_presetChoices->clear();
    for (const QString &project : recentProjects()) {
        const QDir presets(QDir(project).filePath(QStringLiteral("presets")));
        for (const QFileInfo &file : presets.entryInfoList({QStringLiteral("*.kerfpreset")}, QDir::Files, QDir::Name)) {
            auto *item = new QListWidgetItem(QStringLiteral("%1\nfrom %2").arg(file.completeBaseName(), QFileInfo(project).fileName()));
            item->setData(kPathRole, file.absoluteFilePath());
            item->setToolTip(file.absoluteFilePath());
            m_presetChoices->addItem(item);
        }
    }
    if (m_presetChoices->count() == 0) {
        auto *empty = new QListWidgetItem(QStringLiteral("Presets saved in your projects' presets/ folders appear here."));
        empty->setFlags(Qt::NoItemFlags);
        m_presetChoices->addItem(empty);
    }
    // The grouped list is as tall as its rows, up to three of them.
    m_presetChoices->setFixedHeight(std::clamp(m_presetChoices->count(), 1, 3) * RowDelegate::rowHeight(RowDelegate::PresetRow) + 2);
}

void ProjectLauncher::loadPreset(const QString &path) {
    showPreset(path, m_inspectPreset(QJsonObject{{QStringLiteral("path"), path}}));
}

void ProjectLauncher::showPreset(const QString &path, const QJsonObject &preset) {
    while (m_presetSlotsLayout->count() > 1) {
        QLayoutItem *item = m_presetSlotsLayout->takeAt(0);
        if (QWidget *widget = item->widget()) widget->deleteLater();
        delete item;
    }
    m_slotOrder.clear();
    m_slotFiles.clear();
    m_slotFits.clear();
    m_slotStatus.clear();
    m_slotStarts.clear();
    m_presetTexts.clear();
    m_presetPath = path;
    const QString error = preset.value(QStringLiteral("error")).toString();
    m_preset = error.isEmpty() ? preset : QJsonObject{};
    if (!error.isEmpty()) {
        m_presetTitle->setText(QStringLiteral("%1 could not be read: %2").arg(QFileInfo(path).fileName(), error));
        updateChoose();
        return;
    }
    const QJsonArray mediaSlots = preset.value(QStringLiteral("slots")).toArray();
    m_presetTitle->setText(QStringLiteral("%1  ·  %2 media slot%3")
                               .arg(preset.value(QStringLiteral("name")).toString(QFileInfo(path).completeBaseName()))
                               .arg(mediaSlots.size())
                               .arg(mediaSlots.size() == 1 ? QString() : QStringLiteral("s")));
    // The slots, then the preset's texts, each as a grouped list.
    int row = 0;
    QFrame *slotGroup = nullptr;
    QVBoxLayout *slotRows = nullptr;
    for (const QJsonValue &value : mediaSlots) {
        const QJsonObject slot = value.toObject();
        const QString id = slot.value(QStringLiteral("id")).toString();
        m_slotOrder.append(id);
        if (!slotGroup) {
            slotGroup = new QFrame;
            slotGroup->setObjectName(QStringLiteral("presetSlotGroup"));
            slotRows = new QVBoxLayout(slotGroup);
            slotRows->setContentsMargins(0, 0, 0, 0);
            slotRows->setSpacing(0);
            m_presetSlotsLayout->insertWidget(row++, slotGroup);
        } else {
            slotRows->addWidget(groupSeparator());
        }
        auto *frame = new QFrame;
        frame->setObjectName(QStringLiteral("presetSlot"));
        auto *grid = new QGridLayout(frame);
        grid->setContentsMargins(14, 9, 12, 9);
        grid->setHorizontalSpacing(12);
        grid->setVerticalSpacing(1);
        auto *name = new QLabel(id);
        name->setObjectName(QStringLiteral("presetSlotName"));
        grid->addWidget(name, 0, 0);
        const bool video = slot.value(QStringLiteral("video")).toBool();
        const bool audio = slot.value(QStringLiteral("audio")).toBool();
        QStringList needs{QStringLiteral("%1 s").arg(slot.value(QStringLiteral("seconds")).toDouble(), 0, 'f', 2)};
        if (video || audio)
            needs << (video && audio ? QStringLiteral("picture and sound") : video ? QStringLiteral("picture") : QStringLiteral("sound"));
        const int clips = slot.value(QStringLiteral("clips")).toInt();
        if (clips > 0) needs << (clips == 1 ? QStringLiteral("1 clip") : QStringLiteral("%1 clips").arg(clips));
        auto *detail = new QLabel(QStringLiteral("Needs ") + needs.join(QStringLiteral(" · ")));
        detail->setObjectName(QStringLiteral("launcherHint"));
        grid->addWidget(detail, 1, 0);
        auto *status = new QLabel(QStringLiteral("No file chosen"));
        status->setObjectName(QStringLiteral("presetSlotStatus_%1").arg(id));
        status->setProperty("fit", QStringLiteral("none"));
        status->setWordWrap(true);
        grid->addWidget(status, 0, 1, 2, 1);
        auto *start = new QDoubleSpinBox;
        start->setObjectName(QStringLiteral("presetSlotFrom_%1").arg(id));
        start->setPrefix(QStringLiteral("from "));
        start->setSuffix(QStringLiteral(" s"));
        start->setDecimals(2);
        start->setRange(0, 0);
        start->setEnabled(false);
        start->setButtonSymbols(QAbstractSpinBox::NoButtons);
        start->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
        start->setMinimumWidth(96);
        start->setToolTip(QStringLiteral("Where the slot starts in the chosen file"));
        grid->addWidget(start, 0, 2, 2, 1);
        auto *pick = new QPushButton(QStringLiteral("Choose File…"));
        pick->setObjectName(QStringLiteral("presetSlotFile_%1").arg(id));
        connect(pick, &QPushButton::clicked, this, [this, id] { chooseSlotFile(id); });
        grid->addWidget(pick, 0, 3, 2, 1);
        grid->setColumnStretch(0, 1);
        grid->setColumnStretch(1, 1);
        m_slotStatus.insert(id, status);
        m_slotStarts.insert(id, start);
        slotRows->addWidget(frame);
    }
    const QJsonObject texts = preset.value(QStringLiteral("texts")).toObject();
    if (!texts.isEmpty()) {
        auto *frame = new QFrame;
        frame->setObjectName(QStringLiteral("presetSlotGroup"));
        auto *form = launcherForm(frame);
        form->setContentsMargins(14, 10, 12, 10);
        for (auto it = texts.begin(); it != texts.end(); ++it) {
            auto *text = new QLineEdit(it.value().toString());
            text->setObjectName(QStringLiteral("presetText_%1").arg(it.key()));
            form->addRow(formLabel(QStringLiteral("Text %1").arg(it.key())), text);
            m_presetTexts.insert(it.key(), text);
        }
        m_presetSlotsLayout->insertWidget(row, frame);
    }
    updateDetailsSummary();
    updateChoose();
}

void ProjectLauncher::chooseSlotFile(const QString &slot) {
    const QString start = m_slotFiles.isEmpty() ? QDir::homePath() : QFileInfo(m_slotFiles.cbegin().value()).absolutePath();
    const QString file = QFileDialog::getOpenFileName(
        this, QStringLiteral("Media for %1").arg(slot), start,
        QStringLiteral("Media files (*.mov *.mp4 *.m4v *.mkv *.webm *.wav *.mp3 *.aif *.aiff);;All files (*)"));
    if (!file.isEmpty()) setSlotFile(slot, file);
}

void ProjectLauncher::setSlotFile(const QString &slot, const QString &file) {
    m_slotFiles.insert(slot, file);
    refit();
}

void ProjectLauncher::refit() {
    QStringList files;
    for (const QString &slot : std::as_const(m_slotOrder))
        if (m_slotFiles.contains(slot) && !files.contains(m_slotFiles.value(slot))) files << m_slotFiles.value(slot);
    if (files.isEmpty()) return;
    QGuiApplication::setOverrideCursor(Qt::WaitCursor);
    const QJsonObject answer = m_inspectPreset(QJsonObject{{QStringLiteral("path"), m_presetPath},
                                                           {QStringLiteral("files"), QJsonArray::fromStringList(files)}});
    QGuiApplication::restoreOverrideCursor();
    const QJsonArray probed = answer.value(QStringLiteral("files")).toArray();
    QHash<QString, QJsonObject> needs;
    for (const QJsonValue &value : m_preset.value(QStringLiteral("slots")).toArray())
        needs.insert(value.toObject().value(QStringLiteral("id")).toString(), value.toObject());
    for (const QString &slot : std::as_const(m_slotOrder)) {
        QLabel *status = m_slotStatus.value(slot);
        if (!status || !m_slotFiles.contains(slot)) continue;
        const QString file = m_slotFiles.value(slot);
        const QString name = QFileInfo(file).fileName();
        QJsonObject entry;
        for (const QJsonValue &value : probed)
            if (QDir::cleanPath(value.toObject().value(QStringLiteral("path")).toString()) == QDir::cleanPath(file))
                entry = value.toObject();
        if (entry.isEmpty() && files.indexOf(file) < probed.size()) entry = probed.at(files.indexOf(file)).toObject();
        bool fits = false;
        QString text;
        if (answer.contains(QStringLiteral("error"))) {
            text = QStringLiteral("✕ %1 could not be checked: %2").arg(name, answer.value(QStringLiteral("error")).toString());
        } else if (entry.isEmpty()) {
            text = QStringLiteral("✕ Kerf could not read %1").arg(name);
        } else if (entry.contains(QStringLiteral("error"))) {
            text = QStringLiteral("✕ %1: %2").arg(name, entry.value(QStringLiteral("error")).toString());
        } else if (entry.value(QStringLiteral("fits")).toArray().contains(slot)) {
            fits = true;
            text = QStringLiteral("✓ %1 · %2 s").arg(name).arg(entry.value(QStringLiteral("seconds")).toDouble(), 0, 'f', 2);
        } else {
            const QJsonObject need = needs.value(slot);
            const QString why = need.value(QStringLiteral("video")).toBool() && !entry.value(QStringLiteral("video")).toBool()
                                    ? QStringLiteral("has no picture")
                                : need.value(QStringLiteral("audio")).toBool() && !entry.value(QStringLiteral("audio")).toBool()
                                    ? QStringLiteral("has no sound")
                                    : QStringLiteral("is shorter than %1 s").arg(need.value(QStringLiteral("seconds")).toDouble(), 0, 'f', 2);
            text = QStringLiteral("✕ %1 %2").arg(name, why);
        }
        m_slotFits.insert(slot, fits);
        // The file's length past what the slot plays; kerfd checks the
        // transition handles when it applies the preset.
        QDoubleSpinBox *start = m_slotStarts.value(slot);
        const double spare = entry.value(QStringLiteral("seconds")).toDouble() - needs.value(slot).value(QStringLiteral("seconds")).toDouble();
        start->setMaximum(fits ? std::max(0.0, spare) : 0.0);
        start->setEnabled(fits && spare > 0);
        status->setText(text);
        status->setToolTip(file);
        status->setProperty("fit", fits ? QStringLiteral("yes") : QStringLiteral("no"));
        repolish(status);
    }
    updateChoose();
}

void ProjectLauncher::setTools(const QJsonObject &tools) {
    m_tools = tools;
    const bool git = tools.value(QStringLiteral("git")).toBool();
    const QString user = tools.value(QStringLiteral("ghUser")).toString();
    const bool signedIn = tools.value(QStringLiteral("gh")).toBool() && !user.isEmpty();
    m_git->setEnabled(git);
    if (!git) m_git->setChecked(false);
    m_github->setEnabled(signedIn);
    if (!signedIn) m_github->setChecked(false);
    m_remote->setEnabled(git && !m_github->isChecked());
    QStringList hints;
    if (!git) {
        hints << QStringLiteral("Install Git to share projects (xcode-select --install).");
    } else {
        hints << (signedIn ? QStringLiteral("Signed in to GitHub as @%1.").arg(user)
                  : tools.value(QStringLiteral("gh")).toBool()
                      ? QStringLiteral("Sign in with `gh auth login` to create a GitHub repository here.")
                      : QStringLiteral("Install the GitHub CLI (brew install gh) to create a GitHub repository here."));
        if (!tools.value(QStringLiteral("gitLfs")).toBool())
            hints << QStringLiteral("Git LFS is not installed: media of 100 MB or more cannot be shared.");
    }
    m_gitHint->setText(hints.join(QLatin1Char(' ')));
    updateChoose();
}

void ProjectLauncher::openExisting() {
    const QString path = QFileDialog::getExistingDirectory(this, QStringLiteral("Open an Existing Project"),
                                                           QFileInfo(defaultLocation()).absolutePath());
    if (path.isEmpty()) return;
    if (!QFileInfo::exists(QDir(path).filePath(QStringLiteral("project.kerf"))) &&
        QMessageBox::question(this, QStringLiteral("Open Project"),
                              QStringLiteral("“%1” has no project.kerf. Start a new project in it?").arg(QFileInfo(path).fileName())) !=
            QMessageBox::Yes)
        return;
    m_chosen = path;
    accept();
}

QString ProjectLauncher::templateId() const {
    return m_templates->currentItem() ? m_templates->currentItem()->data(kPathRole).toString() : QStringLiteral("standard");
}

QString ProjectLauncher::projectFolder() const {
    const QString name = folderName(m_name->text());
    const QString location = expanded(m_location->text());
    if (name.isEmpty() || location.isEmpty()) return {};
    return QDir::cleanPath(QDir(location).filePath(name));
}

QString ProjectLauncher::cloneFolder() const {
    QString url = m_cloneUrl->text().trimmed();
    const QString location = expanded(m_cloneLocation->text());
    while (url.endsWith(QLatin1Char('/'))) url.chop(1);
    QString name = url.mid(std::max(url.lastIndexOf(QLatin1Char('/')), url.lastIndexOf(QLatin1Char(':'))) + 1);
    if (name.endsWith(QStringLiteral(".git"))) name.chop(4);
    if (name.isEmpty() || location.isEmpty()) return {};
    return QDir::cleanPath(QDir(location).filePath(name));
}

QString ProjectLauncher::detailsProblem() const {
    if (folderName(m_name->text()).isEmpty()) return QStringLiteral("Name the project");
    const QString location = expanded(m_location->text());
    if (location.isEmpty()) return QStringLiteral("Choose where to save the project");
    if (QDir::isRelativePath(location)) return QStringLiteral("Choose a full location, such as ~/Movies/Kerf");
    if (occupied(projectFolder()))
        return QStringLiteral("“%1” is already in that location; choose another name").arg(folderName(m_name->text()));
    const QString remote = m_remote->text().trimmed();
    if (!remote.isEmpty() && !isGitAddress(remote)) return QStringLiteral("The remote must be a Git address");
    return {};
}

QString ProjectLauncher::problem() const {
    switch (m_pages->currentIndex()) {
    case NewProject:
        if (!m_templates->currentItem()) return QStringLiteral("Choose a template");
        return detailsProblem();
    case Recent: {
        const QListWidgetItem *item = m_recent->currentItem();
        if (!item || !item->flags().testFlag(Qt::ItemIsEnabled)) return QStringLiteral("Choose a recent project");
        if (!item->data(kExistsRole).toBool()) return QStringLiteral("That project folder is no longer there");
        return {};
    }
    case FromPreset:
        if (m_preset.isEmpty()) return QStringLiteral("Choose a preset");
        for (const QString &slot : m_slotOrder)
            if (!m_slotFiles.contains(slot)) return QStringLiteral("Choose a file for each media slot");
        for (const QString &slot : m_slotOrder)
            if (!m_slotFits.value(slot))
                return QStringLiteral("%1 cannot fill %2").arg(QFileInfo(m_slotFiles.value(slot)).fileName(), slot);
        return detailsProblem();
    case Clone: {
        const QString url = m_cloneUrl->text().trimmed();
        if (url.isEmpty()) return QStringLiteral("Enter the repository's address");
        if (!isGitAddress(url)) return QStringLiteral("Enter a Git address such as https://github.com/you/film.git");
        const QString location = expanded(m_cloneLocation->text());
        if (location.isEmpty() || QDir::isRelativePath(location)) return QStringLiteral("Choose where to put the copy");
        if (occupied(cloneFolder())) return QStringLiteral("“%1” is already in that location").arg(QFileInfo(cloneFolder()).fileName());
        return {};
    }
    }
    return {};
}

void ProjectLauncher::updateChoose() {
    if (!m_choose || !m_pages) return;
    const QString why = m_busy ? QString() : problem();
    m_choose->setEnabled(!m_busy && why.isEmpty());
    m_choose->setToolTip(why);
    if (m_busy) return;
    m_problem->setText(why);
    m_problem->setProperty("state", QStringLiteral("hint"));
    repolish(m_problem);
}

void ProjectLauncher::updateDetailsSummary() {
    if (!m_detailsSummary || !m_rate || !m_pages) return;
    // What Choose makes, in one line: the name, what it starts from, the
    // frame rate, where, and how it is shared.
    const QString name = m_name->text().trimmed();
    QStringList summary{name.isEmpty() ? QStringLiteral("Create an unnamed project") : QStringLiteral("Create “%1”").arg(name)};
    if (m_pages->currentIndex() == FromPreset) {
        summary << (m_preset.isEmpty() ? QStringLiteral("from a preset")
                                       : QStringLiteral("from %1").arg(m_preset.value(QStringLiteral("name")).toString(
                                             QFileInfo(m_presetPath).completeBaseName())));
    } else {
        for (const TemplateInfo &entry : kTemplates)
            if (templateId() == QLatin1String(entry.id)) summary << QString::fromLatin1(entry.label);
    }
    summary << m_rate->currentText() << homeRelative(expanded(m_location->text()));
    if (m_github->isChecked()) summary << QStringLiteral("private GitHub repository");
    else if (!m_remote->text().trimmed().isEmpty()) summary << QStringLiteral("Git, pushed to the remote");
    else if (m_git->isChecked()) summary << QStringLiteral("Git");
    m_detailsSummary->setText(summary.join(QStringLiteral(" · ")));
    const QString folder = projectFolder();
    m_destination->setText(folder.isEmpty() ? QString() : QStringLiteral("Creates %1").arg(homeRelative(folder)));
}

QJsonObject ProjectLauncher::createRequest() const {
    const QString remote = m_remote->text().trimmed();
    const bool github = m_github->isChecked();
    QJsonObject request{{QStringLiteral("type"), QStringLiteral("create")},
                        {QStringLiteral("name"), m_name->text().trimmed()},
                        {QStringLiteral("location"), expanded(m_location->text())},
                        {QStringLiteral("template"), templateId()},
                        {QStringLiteral("rate"), m_rate->currentData().toString()},
                        {QStringLiteral("git"), m_git->isChecked() || github || !remote.isEmpty()}};
    if (github) request.insert(QStringLiteral("github"), QJsonObject{{QStringLiteral("private"), true}});
    else if (!remote.isEmpty()) request.insert(QStringLiteral("remote"), remote);
    // A bare name is looked up the way the bridge would look it up anyway.
    if (m_kerfd.contains(QLatin1Char('/'))) request.insert(QStringLiteral("kerfd"), QFileInfo(m_kerfd).absoluteFilePath());
    return request;
}

void ProjectLauncher::choose() {
    if (m_busy || !problem().isEmpty()) {
        updateChoose();
        return;
    }
    const int page = m_pages->currentIndex();
    if (page == Recent) {
        m_chosen = m_recent->currentItem()->data(kPathRole).toString();
        accept();
        return;
    }
    QJsonObject request;
    QString message;
    if (page == Clone) {
        request = QJsonObject{{QStringLiteral("type"), QStringLiteral("clone")},
                              {QStringLiteral("url"), m_cloneUrl->text().trimmed()},
                              {QStringLiteral("location"), expanded(m_cloneLocation->text())}};
        message = QStringLiteral("Cloning %1…").arg(QFileInfo(cloneFolder()).fileName());
    } else {
        request = createRequest();
        message = QStringLiteral("Making %1…").arg(folderName(m_name->text()));
        QSettings settings;
        settings.setValue(QStringLiteral("projects/template"), templateId());
        settings.setValue(QStringLiteral("projects/rate"), m_rate->currentData().toString());
        if (page == FromPreset) {
            QJsonObject bindings;
            for (const QString &slot : std::as_const(m_slotOrder)) {
                QJsonObject binding{{QStringLiteral("file"), m_slotFiles.value(slot)}};
                const double from = m_slotStarts.value(slot)->value();
                if (from > 0) binding.insert(QStringLiteral("from"), QStringLiteral("%1s").arg(from, 0, 'f', 2));
                bindings.insert(slot, binding);
            }
            QJsonObject preset{{QStringLiteral("path"), m_presetPath}, {QStringLiteral("bindings"), bindings}};
            QJsonObject texts;
            for (auto it = m_presetTexts.cbegin(); it != m_presetTexts.cend(); ++it)
                if (it.value()->isModified()) texts.insert(it.key(), it.value()->text());
            if (!texts.isEmpty()) preset.insert(QStringLiteral("texts"), texts);
            request.insert(QStringLiteral("preset"), preset);
            message = QStringLiteral("Making %1 from the preset…").arg(folderName(m_name->text()));
        }
    }
    setBusy(true, message);
    runTask(request, [this](const QJsonObject &status) { handleTaskStatus(status); });
}

void ProjectLauncher::runTask(const QJsonObject &request, std::function<void(const QJsonObject &)> update) {
    m_requests.append(request);
#ifdef KERF_QT_TESTING
    Q_UNUSED(update);
#else
    const QByteArray json = QJsonDocument(request).toJson(QJsonDocument::Compact);
    void *task = kerf_task_start(json.constData());
    if (!task) {
        update(QJsonObject{{QStringLiteral("status"), QStringLiteral("failed")},
                           {QStringLiteral("error"), QStringLiteral("Kerf could not start the task.")}});
        return;
    }
    auto *timer = new QTimer(this);
    timer->setInterval(100);
    m_tasks.insert(timer, task);
    connect(timer, &QTimer::timeout, this, [this, timer, update] {
        void *running = m_tasks.value(timer);
        char *raw = kerf_task_poll(running);
        QJsonObject status{{QStringLiteral("status"), QStringLiteral("failed")},
                           {QStringLiteral("error"), QStringLiteral("The task stopped answering.")}};
        if (raw) {
            const QJsonDocument document = QJsonDocument::fromJson(QByteArray(raw));
            kerf_bridge_string_free(raw);
            if (document.isObject()) status = document.object();
        }
        if (status.value(QStringLiteral("status")).toString() != QStringLiteral("running")) {
            timer->stop();
            m_tasks.remove(timer);
            kerf_task_free(running);
            timer->deleteLater();
        }
        update(status);
    });
    timer->start();
#endif
}

void ProjectLauncher::handleTaskStatus(const QJsonObject &status) {
    const QString state = status.value(QStringLiteral("status")).toString();
    if (state == QStringLiteral("running")) {
        const QString message = status.value(QStringLiteral("message")).toString();
        if (!message.isEmpty()) m_problem->setText(message);
        return;
    }
    setBusy(false);
    if (state != QStringLiteral("done")) {
        showFailure(status.value(QStringLiteral("error")).toString(QStringLiteral("Kerf could not make the project.")));
        return;
    }
    const QString path = status.value(QStringLiteral("path")).toString();
    if (path.isEmpty()) {
        showFailure(QStringLiteral("The task finished without a project folder."));
        return;
    }
    const bool cloned = m_pages->currentIndex() == Clone;
    QSettings().setValue(QStringLiteral("projects/location"), expanded((cloned ? m_cloneLocation : m_location)->text()));
    QStringList warnings;
    for (const QJsonValue &warning : status.value(QStringLiteral("warnings")).toArray()) warnings << warning.toString();
#ifndef KERF_QT_TESTING
    if (!warnings.isEmpty())
        QMessageBox::warning(this, QStringLiteral("Project ready"), warnings.join(QStringLiteral("\n\n")));
#endif
    m_chosen = path;
    accept();
}

void ProjectLauncher::setBusy(bool busy, const QString &message) {
    m_busy = busy;
    m_progress->setVisible(busy);
    m_sidebar->setEnabled(!busy);
    m_pages->setEnabled(!busy);
    m_details->setEnabled(!busy);
    m_openExisting->setEnabled(!busy);
    if (busy) {
        m_problem->setText(message);
        m_problem->setProperty("state", QStringLiteral("busy"));
        repolish(m_problem);
    }
    updateChoose();
}

void ProjectLauncher::showFailure(const QString &message) {
    m_problem->setText(message);
    m_problem->setProperty("state", QStringLiteral("error"));
    repolish(m_problem);
}

} // namespace kerf
