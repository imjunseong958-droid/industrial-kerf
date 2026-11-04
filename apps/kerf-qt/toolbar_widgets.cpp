#include "toolbar_widgets.h"

#include "editor_common.h"
#include "ui/icons.h"

#include <QAction>
#include <QActionGroup>
#include <QEnterEvent>
#include <QFocusEvent>
#include <QFontMetricsF>
#include <QHBoxLayout>
#include <QKeyEvent>
#include <QKeySequence>
#include <QLocale>
#include <QMenu>
#include <QPainter>
#include <QPainterPath>
#include <QResizeEvent>
#include <QVariantAnimation>

#include <algorithm>
#include <cmath>

namespace kerf {

using namespace tokens;

// ================================================================ helpers

QString frameRateLabel(double fps) {
    if (!std::isfinite(fps) || fps <= 0) return QString();
    const double whole = std::round(fps);
    if (std::abs(fps - whole) < 0.005) return QStringLiteral("%1p").arg(qint64(whole));
    return QStringLiteral("%1p").arg(QString::number(fps, 'f', 2));
}

QString sequenceLabel(const QSize &size, double fps) {
    QStringList parts;
    if (size.isValid() && !size.isEmpty()) parts << QStringLiteral("%1×%2").arg(size.width()).arg(size.height());
    if (const QString rate = frameRateLabel(fps); !rate.isEmpty()) parts << rate;
    return parts.join(QStringLiteral(" · "));
}

QString editedAgo(const QDateTime &then, const QDateTime &now) {
    if (!then.isValid() || !now.isValid()) return QString();
    const qint64 seconds = std::max<qint64>(0, then.secsTo(now));
    if (seconds < 60) return QStringLiteral("just now");
    const qint64 minutes = seconds / 60;
    if (minutes < 60) return QStringLiteral("%1 min ago").arg(minutes);
    const qint64 hours = minutes / 60;
    const qint64 days = then.toLocalTime().date().daysTo(now.toLocalTime().date());
    if (hours < 24 && days == 0) return QStringLiteral("%1 h ago").arg(hours);
    if (days <= 1) return QStringLiteral("yesterday");
    if (days < 7) return QStringLiteral("%1 days ago").arg(days);
    return QLocale(QLocale::English).toString(then.toLocalTime().date(), QStringLiteral("d MMM yyyy"));
}

QString formatName(int presetIndex) {
    const QList<CanvasPreset> presets = canvasPresets();
    if (presetIndex < 0 || presetIndex >= presets.size()) return QString();
    static const char *const names[] = {"Landscape", "Vertical", "Square", "Portrait"};
    const QString ratio = QString::fromLatin1(presets.at(presetIndex).ratio);
    return presetIndex < 4 ? QStringLiteral("%1 %2").arg(QString::fromLatin1(names[presetIndex]), ratio) : ratio;
}

QString formatTip(int presetIndex) {
    const QList<CanvasPreset> presets = canvasPresets();
    if (presetIndex < 0 || presetIndex >= presets.size()) return QString();
    const CanvasPreset &preset = presets.at(presetIndex);
    return QStringLiteral("%1 · %2×%3").arg(formatName(presetIndex)).arg(preset.width).arg(preset.height);
}

namespace {

// box-shadow: inset 0 -1px 0 (bottom) or inset 0 1px 0 (top) in `line`.
void paintEdgeLine(QPainter &p, const QRect &rect, bool top) {
    p.fillRect(QRect(rect.left(), top ? rect.top() : rect.bottom(), rect.width(), 1), color(line));
}

// A segment of ToolbarSegments: it paints only its label or icon; the
// control paints the track and the sliding fill behind it.
class ToolbarSegment final : public ui::KitButton {
public:
    ToolbarSegment(QWidget *parent, QString text, QString icon)
        : ui::KitButton(parent), m_text(std::move(text)), m_icon(std::move(icon)) {
        setFocusPolicy(Qt::NoFocus);
        setText(m_text);
        setAccessibleName(m_text);
    }
    QString label() const { return m_text; }
    bool isIcon() const { return !m_icon.isEmpty(); }
    QString iconName() const { return m_icon; }
    void setOn(bool on) {
        if (m_on == on) return;
        m_on = on;
        update();
    }
    bool isOn() const { return m_on; }
    QColor labelColor() const {
        if (!isEnabled()) return color(dis);
        if (m_on) return color(ink);
        return ui::mix(color(ink2), color(ink), hoverProgress());
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        const QColor c = labelColor();
        if (isIcon()) {
            icons::paint(p, m_icon, rect(), c, 16);
        } else {
            p.setFont(tokens::font(m_on ? controlStrong : control));
            p.setPen(c);
            p.drawText(rect(), Qt::AlignCenter, m_text);
        }
    }

private:
    QString m_text;
    QString m_icon;
    bool m_on = false;
};

ToolbarSegment *asSegment(QWidget *widget) { return static_cast<ToolbarSegment *>(widget); }

// The Export button's parts: the main part paints "Export" and the quality,
// the arrow part its chevron. The button paints the outline and divider.
class ExportPart final : public ui::KitButton {
public:
    ExportPart(QWidget *parent, bool arrow) : ui::KitButton(parent), m_arrow(arrow) { setFocusPolicy(Qt::TabFocus); }
    void setQualityName(const QString &name) {
        m_quality = name;
        update();
    }
    QString qualityName() const { return m_quality; }
    // pad 0 10, "Export" (UI 500) gap 6 the quality (UI 400).
    int naturalWidth() const {
        return space::s8 + 2 + ui::snapWidth(controlStrong, QStringLiteral("Export")) + space::s6 +
               std::max(ui::snapWidth(control, QStringLiteral("Master")), ui::snapWidth(control, QStringLiteral("Draft"))) +
               space::s8 + 2;
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        const QRectF r = rect();
        if (isEnabled() && hoverProgress() > 0) {
            QColor fill = color(raised);
            fill.setAlphaF(float(hoverProgress()));
            QPainterPath path;
            const qreal radiusPx = radius::control;
            // radius 6 0 0 6 on the main part, 0 6 6 0 on the arrow.
            path.addRoundedRect(r, radiusPx, radiusPx);
            QPainterPath square;
            square.addRect(m_arrow ? QRectF(r.left(), r.top(), r.width() / 2, r.height())
                                   : QRectF(r.center().x(), r.top(), r.width() / 2, r.height()));
            path = path.united(square);
            p.fillPath(path, fill);
        }
        if (m_arrow) {
            icons::paint(p, QStringLiteral("g-chev"), r, isEnabled() ? ui::mix(color(ink2), color(ink), hoverProgress()) : color(dis), 16);
        } else {
            const QFont strong = tokens::font(controlStrong);
            const QFont regular = tokens::font(control);
            const int exportWidth = ui::snapWidth(controlStrong, QStringLiteral("Export"));
            const int total = exportWidth + space::s6 + ui::snapWidth(control, m_quality);
            qreal x = std::round((r.width() - total) / 2.0);
            p.setFont(strong);
            p.setPen(isEnabled() ? color(ink) : color(dis));
            p.drawText(QRectF(x, 0, exportWidth + 1, r.height()), Qt::AlignVCenter | Qt::AlignLeft, QStringLiteral("Export"));
            x += exportWidth + space::s6;
            p.setFont(regular);
            p.setPen(isEnabled() ? color(ink3) : color(dis));
            p.drawText(QRectF(x, 0, r.width() - x, r.height()), Qt::AlignVCenter | Qt::AlignLeft, m_quality);
        }
        if (focusVisible()) ui::paintFocusRing(p, r.adjusted(1, 1, -1, -1), radius::control - 1);
    }

private:
    bool m_arrow;
    QString m_quality;
};

} // namespace

// ================================================================ FlatToolBar

FlatToolBar::FlatToolBar(const QString &title, QWidget *parent) : QToolBar(title, parent) {
    setObjectName(QStringLiteral("mainToolbar"));
    setMovable(false);
    setFloatable(false);
    // The app style sheet's QToolBar padding and spacing are for other
    // toolbars; this one lays its row over itself.
    setStyleSheet(QStringLiteral("QToolBar { padding: 0px; spacing: 0px; border: 0px; background: transparent; }"));
    setContextMenuPolicy(Qt::PreventContextMenu);
}

void FlatToolBar::setContent(QWidget *content) {
    m_content = content;
    if (!content) return;
    content->setParent(this);
    content->setGeometry(rect());
    content->show();
}

void FlatToolBar::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.fillRect(rect(), color(chrome));
    paintEdgeLine(p, rect(), false);
}

void FlatToolBar::resizeEvent(QResizeEvent *event) {
    QToolBar::resizeEvent(event);
    if (m_content) m_content->setGeometry(rect());
}

// ================================================================ ToolbarSegments

ToolbarSegments::ToolbarSegments(QWidget *parent) : QWidget(parent) {
    setFocusPolicy(Qt::TabFocus);
    setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Fixed);
    m_slide = new QVariantAnimation(this);
    m_slide->setEasingCurve(easing());
    connect(m_slide, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) {
        m_fill = value.toRectF();
        update();
    });
    relayout();
}

int ToolbarSegments::add(const QString &text, const QString &icon, const QString &tip, const QString &shortcut) {
    auto *button = new ToolbarSegment(this, text, icon);
    const int index = int(m_segments.size());
    m_segments << button;
    if (!tip.isEmpty()) {
        button->setTip(tip, shortcut);
        button->setAccessibleName(tip);
    }
    connect(button, &QAbstractButton::clicked, this, [this, index] {
        setCurrentIndex(index);
        emit activated(index);
    });
    relayout();
    return index;
}

int ToolbarSegments::addSegment(const QString &text, const QString &tip, const QString &shortcut) { return add(text, {}, tip, shortcut); }

int ToolbarSegments::addIconSegment(const QString &icon, const QString &tip, const QString &shortcut) {
    return add({}, icon, tip, shortcut);
}

QString ToolbarSegments::segmentText(int index) const {
    return index >= 0 && index < count() ? asSegment(m_segments[index])->label() : QString();
}

void ToolbarSegments::setSegmentTip(int index, const QString &tip, const QString &shortcut) {
    if (index < 0 || index >= count()) return;
    m_segments[index]->setTip(tip, shortcut);
    m_segments[index]->setAccessibleName(tip);
}

void ToolbarSegments::setIconWidth(int width) {
    m_iconWidth = width;
    relayout();
}

void ToolbarSegments::relayout() {
    constexpr int inner = kHeight - 4;
    int x = 2;
    for (ui::KitButton *button : std::as_const(m_segments)) {
        auto *segment = asSegment(button);
        // The label's width at weight 500, so the on segment never grows.
        const int w = segment->isIcon() ? m_iconWidth : space::s8 + ui::snapWidth(controlStrong, segment->label()) + space::s8;
        segment->setGeometry(x, 2, w, inner);
        x += w;
    }
    setFixedSize(x + 2, kHeight);
    if (m_slide->state() != QAbstractAnimation::Running) m_fill = segmentRect(m_current);
    updateGeometry();
    update();
}

QSize ToolbarSegments::sizeHint() const {
    int x = 2;
    for (ui::KitButton *button : m_segments) {
        auto *segment = asSegment(button);
        x += segment->isIcon() ? m_iconWidth : space::s8 + ui::snapWidth(controlStrong, segment->label()) + space::s8;
    }
    return QSize(x + 2, kHeight);
}

QRectF ToolbarSegments::segmentRect(int index) const {
    return index >= 0 && index < count() ? QRectF(m_segments[index]->geometry()) : QRectF();
}

void ToolbarSegments::setCurrentIndex(int index, bool animate) {
    if (index < -1 || index >= count()) index = -1;
    if (index == m_current) return;
    const int previous = m_current;
    m_current = index;
    for (int i = 0; i < count(); ++i) asSegment(m_segments[i])->setOn(i == index);
    const QRectF target = segmentRect(index);
    const int duration = ui::motionDuration(motion::segment);
    // The fill slides from one segment to the next; it appears in place.
    if (animate && previous >= 0 && index >= 0 && isVisible() && duration > 0) {
        m_slide->stop();
        m_slide->setDuration(duration);
        m_slide->setStartValue(m_fill);
        m_slide->setEndValue(target);
        m_slide->start();
    } else {
        m_slide->stop();
        m_fill = target;
        update();
    }
}

void ToolbarSegments::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF track(0, 0, width(), kHeight);
    p.setPen(Qt::NoPen);
    p.setBrush(color(raised));
    p.drawRoundedRect(track, radius::control, radius::control);
    if (m_current >= 0 && !m_fill.isEmpty()) {
        p.setBrush(isEnabled() ? color(on) : color(raised).lighter(108));
        p.drawRoundedRect(m_fill, radius::small, radius::small);
        ui::paintInsetLine(p, m_fill, radius::small, color(line2));
        ui::paintTopHighlight(p, m_fill, radius::small, color(segmentHighlight));
    }
    if (m_focusVisible && hasFocus()) ui::paintFocusRing(p, track, radius::control);
}

void ToolbarSegments::keyPressEvent(QKeyEvent *event) {
    const int step = event->key() == Qt::Key_Left ? -1 : event->key() == Qt::Key_Right ? 1 : 0;
    if (!step) {
        QWidget::keyPressEvent(event);
        return;
    }
    const int from = m_current >= 0 ? m_current : (step > 0 ? -1 : count());
    for (int i = from + step; i >= 0 && i < count(); i += step) {
        if (!m_segments[i]->isEnabled()) continue;
        setCurrentIndex(i);
        emit activated(i);
        break;
    }
}

void ToolbarSegments::focusInEvent(QFocusEvent *event) {
    QWidget::focusInEvent(event);
    m_focusVisible = event->reason() == Qt::TabFocusReason || event->reason() == Qt::BacktabFocusReason;
    update();
}

void ToolbarSegments::focusOutEvent(QFocusEvent *event) {
    QWidget::focusOutEvent(event);
    m_focusVisible = false;
    update();
}

void ToolbarSegments::changeEvent(QEvent *event) {
    QWidget::changeEvent(event);
    if (event->type() == QEvent::EnabledChange) update();
}

// ================================================================ FormatBank

FormatBank::FormatBank(QWidget *parent) : ToolbarSegments(parent) {
    setObjectName(QStringLiteral("formatSegments"));
    setAccessibleName(QStringLiteral("Sequence format"));
    static const char *const glyphs[] = {"g-169", "g-916", "g-11", "g-45"};
    const QList<CanvasPreset> presets = canvasPresets();
    for (int i = 0; i < presets.size(); ++i) {
        const QString glyph = i < 4 ? QString::fromLatin1(glyphs[i]) : QStringLiteral("g-169");
        // The toolbar's ⌥1-4; ToolbarWiring names its actions' keys again.
        addIconSegment(glyph, formatTip(i), QKeySequence(Qt::ALT | Qt::Key(Qt::Key_1 + i)).toString(QKeySequence::NativeText));
    }
    connect(this, &ToolbarSegments::activated, this, [this](int index) {
        const QSize size = format(index);
        if (size.isEmpty() || size == m_size) return;
        m_size = size;
        setToolTip(canvasSizeLabel(size));
        emit formatChosen(size);
    });
    setCanvasSize(format(0));
}

QSize FormatBank::format(int index) const {
    const QList<CanvasPreset> presets = canvasPresets();
    if (index < 0 || index >= presets.size()) return {};
    return QSize(presets.at(index).width, presets.at(index).height);
}

int FormatBank::presetIndex(const QSize &size) {
    const QList<CanvasPreset> presets = canvasPresets();
    for (int i = 0; i < presets.size(); ++i)
        if (QSize(presets.at(i).width, presets.at(i).height) == size) return i;
    return -1;
}

void FormatBank::setCanvasSize(const QSize &size) {
    m_size = size;
    setCurrentIndex(presetIndex(size), isVisible());
    setToolTip(size.isValid() ? canvasSizeLabel(size) : QString());
}

// ================================================================ ExportButton

ExportButton::ExportButton(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("exportButton"));
    setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Fixed);
    auto *main = new ExportPart(this, false);
    main->setObjectName(QStringLiteral("exportMain"));
    main->setAccessibleName(QStringLiteral("Export"));
    m_main = main;
    auto *arrow = new ExportPart(this, true);
    arrow->setObjectName(QStringLiteral("exportOptions"));
    arrow->setAccessibleName(QStringLiteral("Export options"));
    arrow->setTip(QStringLiteral("Export options"));
    m_arrow = arrow;

    m_menu = new QMenu(this);
    m_menu->setObjectName(QStringLiteral("exportMenu"));
    ui::styleMenu(m_menu);
    m_menu->setMinimumWidth(324);
    m_qualities = new QActionGroup(this);
    m_qualities->setExclusive(true);
    m_master = new QAction(QStringLiteral("Master"), this);
    m_master->setObjectName(QStringLiteral("exportMaster"));
    m_master->setData(QStringLiteral("full"));
    m_draft = new QAction(QStringLiteral("Draft"), this);
    m_draft->setObjectName(QStringLiteral("exportDraft"));
    m_draft->setData(QStringLiteral("preview"));
    ui::setMenuKey(m_master, QStringLiteral("ProRes 422 HQ"), true);
    ui::setMenuKey(m_draft, QStringLiteral("H.264 review"), true);
    for (QAction *quality : {m_master, m_draft}) {
        quality->setCheckable(true);
        m_qualities->addAction(quality);
        connect(quality, &QAction::triggered, this, [this, quality] { setQuality(quality->data().toString()); });
    }
    m_master->setChecked(true);
    main->setQualityName(qualityName());

    connect(m_main, &QAbstractButton::clicked, this, [this] { emit exportRequested(exportQuality()); });
    connect(m_arrow, &QAbstractButton::clicked, this, &ExportButton::showMenu);
    relayout();
}

QString ExportButton::exportQuality() const { return m_draft->isChecked() ? QStringLiteral("review") : QStringLiteral("master"); }

QString ExportButton::quality() const { return m_draft->isChecked() ? QStringLiteral("preview") : QStringLiteral("full"); }

QString ExportButton::qualityName() const { return m_draft->isChecked() ? QStringLiteral("Draft") : QStringLiteral("Master"); }

QString ExportButton::codec() const { return m_draft->isChecked() ? QStringLiteral("h264") : QStringLiteral("prores_hq"); }

void ExportButton::setQuality(const QString &quality) {
    (quality == QStringLiteral("review") || quality == QStringLiteral("preview") ? m_draft : m_master)->setChecked(true);
    static_cast<ExportPart *>(m_main)->setQualityName(qualityName());
    m_main->setAccessibleDescription(qualityName());
    if (exportQuality() == m_quality) return;
    m_quality = exportQuality();
    emit qualityChanged(m_quality);
}

void ExportButton::setExportEnabled(bool enabled, const QString &tip, const QString &shortcut) {
    m_main->setEnabled(enabled);
    m_main->setTip(tip, shortcut);
    update();
}

QSize ExportButton::sizeHint() const {
    return QSize(static_cast<ExportPart *>(m_main)->naturalWidth() + kArrowWidth, kHeight);
}

void ExportButton::relayout() {
    const QSize size = sizeHint();
    setFixedSize(size);
    m_main->setGeometry(0, 0, size.width() - kArrowWidth, kHeight);
    m_arrow->setGeometry(size.width() - kArrowWidth, 0, kArrowWidth, kHeight);
}

void ExportButton::showMenu() {
    emit menuAboutToShow();
    ui::popupMenu(m_menu, m_arrow);
}

void ExportButton::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    // inset 0 0 0 1px line2, radius 6; the chevron's divider is an inset
    // 1px line at its left.
    ui::paintInsetLine(p, QRectF(rect()), radius::control, color(line2));
    p.fillRect(QRect(m_arrow->x(), 1, 1, height() - 2), color(line));
}

} // namespace kerf
