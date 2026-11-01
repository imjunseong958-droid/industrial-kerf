#include "toolbar_widgets.h"

#include "editor_common.h"
#include "editor_theme.h"

#include <QAction>
#include <QActionGroup>
#include <QFocusEvent>
#include <QFontMetricsF>
#include <QKeyEvent>
#include <QLinearGradient>
#include <QMenu>
#include <QPainter>
#include <QPainterPath>

#include <algorithm>
#include <cmath>
#include <functional>

namespace kerf {

namespace {

// The toolbar's own pixel snapping, as the hardware primitives snap: a 1px
// line is one device pixel at 1x and 1.25x, two at 2x, and edges land on
// device pixels' edges so they stay crisp at fractional scales.
qreal toolbarHairline(const QPainter &painter) {
    const QTransform t = painter.deviceTransform();
    const qreal scale = std::hypot(t.m11(), t.m12());
    return scale > 0 ? std::max<qreal>(1.0, std::round(scale)) / scale : 1.0;
}

QRectF toolbarSnap(const QPainter &painter, const QRectF &rect) {
    const QTransform t = painter.deviceTransform();
    if (t.type() > QTransform::TxScale) return rect;
    bool invertible = false;
    const QTransform back = t.inverted(&invertible);
    if (!invertible) return rect;
    const QRectF device = t.mapRect(rect);
    return back.mapRect(QRectF(QPointF(std::round(device.left()), std::round(device.top())),
                               QPointF(std::round(device.right()), std::round(device.bottom()))));
}

QColor withAlpha(QColor color, qreal alpha) {
    color.setAlphaF(color.alphaF() * alpha);
    return color;
}

// Whether a focus change rings its control, as the editor's focus halo
// decides: Tab, a shortcut or keyboard navigation ring it; a click, or the
// focus a window hands its first control as it opens, does not; the ring
// comes back with its window, or as a menu closes.
bool ringsOnFocusIn(const QWidget *widget, Qt::FocusReason reason, bool hadRing) {
    if (reason == Qt::TabFocusReason || reason == Qt::BacktabFocusReason || reason == Qt::ShortcutFocusReason) return true;
    if (reason == Qt::ActiveWindowFocusReason || reason == Qt::PopupFocusReason) return hadRing;
    return reason != Qt::MouseFocusReason && widget->window()->testAttribute(Qt::WA_KeyboardFocusChange);
}

bool keepsRingOnFocusOut(Qt::FocusReason reason) {
    return reason == Qt::ActiveWindowFocusReason || reason == Qt::PopupFocusReason;
}

void ringKey(QPainter &painter, const QRectF &rect, qreal radius) {
    const EditorTheme &theme = editorTheme();
    painter.save();
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setBrush(Qt::NoBrush);
    painter.setPen(QPen(theme.focusHalo, 2.0));
    painter.drawRoundedRect(rect.adjusted(1.0, 1.0, -1.0, -1.0), radius, radius);
    painter.setPen(QPen(theme.focusRing, 1.0));
    painter.drawRoundedRect(rect.adjusted(0.5, 0.5, -0.5, -0.5), radius, radius);
    painter.restore();
}

// One part of the export key. It draws only its focus ring: the key
// beneath paints the body, its wash under the pointer and while held.
class ExportPart final : public QAbstractButton {
public:
    ExportPart(QWidget *parent, std::function<void()> openMenu) : QAbstractButton(parent), m_openMenu(std::move(openMenu)) {
        setFocusPolicy(Qt::TabFocus);
        setCursor(Qt::ArrowCursor);
    }

protected:
    void paintEvent(QPaintEvent *) override {
        if (!hasFocus() || !m_ring) return;
        QPainter painter(this);
        ringKey(painter, rect(), 4);
    }
    void keyPressEvent(QKeyEvent *event) override {
        if (event->key() == Qt::Key_Return || event->key() == Qt::Key_Enter) {
            click();
            return;
        }
        if (event->key() == Qt::Key_Down && m_openMenu) {
            m_openMenu();
            return;
        }
        QAbstractButton::keyPressEvent(event);
    }
    void focusInEvent(QFocusEvent *event) override {
        m_ring = ringsOnFocusIn(this, event->reason(), m_ring);
        QAbstractButton::focusInEvent(event);
    }
    void focusOutEvent(QFocusEvent *event) override {
        if (!keepsRingOnFocusOut(event->reason())) m_ring = false;
        QAbstractButton::focusOutEvent(event);
    }
    // The key beneath washes the part under the pointer.
    void enterEvent(QEnterEvent *event) override {
        update();
        QAbstractButton::enterEvent(event);
    }
    void leaveEvent(QEvent *event) override {
        update();
        QAbstractButton::leaveEvent(event);
    }

private:
    std::function<void()> m_openMenu;
    bool m_ring = false;
};

} // namespace

// ------------------------------------------------------------ FaceplateToolBar

FaceplateToolBar::FaceplateToolBar(const QString &title, QWidget *parent) : QToolBar(title, parent) {}

void FaceplateToolBar::paintEvent(QPaintEvent *) {
    // The plate itself, not the style sheet's flat chrome: the faceplate's
    // tokens are a gradient and two 1px lines no sheet can name.
    const EditorTheme &t = editorTheme();
    QPainter painter(this);
    const qreal line = toolbarHairline(painter);
    QLinearGradient plate(QPointF(0, 0), QPointF(0, height()));
    plate.setColorAt(0, t.faceplateTop);
    plate.setColorAt(1, t.faceplateBottom);
    painter.fillRect(rect(), plate);
    painter.fillRect(QRectF(0, 0, width(), line), t.faceplateHighlight);
    painter.fillRect(QRectF(0, height() - line, width(), line), t.faceplateEdge);
}

// ------------------------------------------------------------------ FormatKey

FormatKey::FormatKey(const QString &ratio, const QSize &format, QWidget *parent) : QAbstractButton(parent), m_format(format) {
    // Checkable and exclusive, so assistive technology reads the bank as
    // radio buttons and the latched key as chosen.
    setCheckable(true);
    setAutoExclusive(true);
    setFocusPolicy(Qt::TabFocus);
    setFixedSize(kWidth, kHeight);
    setAttribute(Qt::WA_Hover);
    const QString label = canvasSizeLabel(format);
    setToolTip(label);
    setAccessibleName(QStringLiteral("Format %1").arg(label));
    setObjectName(QStringLiteral("formatKey_%1").arg(ratio));
}

QSize FormatKey::sizeHint() const {
    return QSize(kWidth, kHeight);
}

QRectF FormatKey::shapeRect() const {
    // The format's shape fits a 14 × 9 box under the LED, so 16:9 is wide
    // and short and 9:16 tall and narrow, each as large as the box allows.
    constexpr qreal boxWidth = 14;
    constexpr qreal boxHeight = 9;
    const qreal aspect = m_format.height() > 0 ? qreal(m_format.width()) / m_format.height() : 1.0;
    const qreal w = std::round(std::min(boxWidth, boxHeight * aspect));
    const qreal h = std::round(std::min(boxHeight, boxWidth / aspect));
    return QRectF(std::round((width() - w) / 2), std::round(11.5 - h / 2), w, h);
}

void FormatKey::paintEvent(QPaintEvent *) {
    const EditorTheme &t = editorTheme();
    QPainter painter(this);
    if (!isEnabled()) painter.setOpacity(0.5);
    // Held in while chosen, and while the pointer holds it down; lit only
    // when it is the project's format.
    const bool latched = isChecked() || isDown();
    paintRaisedKey(painter, rect(), latched, underMouse() && isEnabled() && !latched, isChecked());
    // The outline: the shape less its inside, a device pixel or two thick.
    painter.setRenderHint(QPainter::Antialiasing);
    const qreal line = toolbarHairline(painter);
    const QRectF outer = toolbarSnap(painter, shapeRect());
    QPainterPath outline;
    outline.addRect(outer);
    QPainterPath inside;
    inside.addRect(outer.adjusted(line, line, -line, -line));
    painter.fillPath(outline.subtracted(inside), t.engravedInk);
    if (showsFocusRing()) ringKey(painter, rect(), 4);
}

void FormatKey::keyPressEvent(QKeyEvent *event) {
    switch (event->key()) {
    case Qt::Key_Left:
    case Qt::Key_Up:
        emit stepRequested(-1);
        return;
    case Qt::Key_Right:
    case Qt::Key_Down:
        emit stepRequested(1);
        return;
    case Qt::Key_Return:
    case Qt::Key_Enter:
        click();
        return;
    default:
        // Space presses and releases the key, as any button's.
        QAbstractButton::keyPressEvent(event);
    }
}

void FormatKey::focusInEvent(QFocusEvent *event) {
    m_ring = ringsOnFocusIn(this, event->reason(), m_ring);
    QAbstractButton::focusInEvent(event);
}

void FormatKey::focusOutEvent(QFocusEvent *event) {
    if (!keepsRingOnFocusOut(event->reason())) m_ring = false;
    QAbstractButton::focusOutEvent(event);
}

// ----------------------------------------------------------------- FormatBank

namespace {
constexpr int kBankPadding = 2;
constexpr int kKeyGap = 2;
constexpr int kLabelGap = 6;
const QString kFormatLabel = QStringLiteral("Format");
} // namespace

FormatBank::FormatBank(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("formatBank"));
    setAccessibleName(QStringLiteral("Format"));
    setFocusPolicy(Qt::NoFocus);
    for (const CanvasPreset &preset : canvasPresets()) {
        auto *key = new FormatKey(QString::fromLatin1(preset.ratio), QSize(preset.width, preset.height), this);
        connect(key, &QAbstractButton::clicked, this, [this, key] { choose(key); });
        connect(key, &FormatKey::stepRequested, this, [this, key](int step) {
            const int at = int(m_keys.indexOf(key)) + step;
            if (at >= 0 && at < m_keys.size()) m_keys.at(at)->setFocus(Qt::TabFocusReason);
        });
        m_keys.append(key);
    }
    setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Fixed);
    setFixedHeight(kHeight);
    setCanvasSize(QSize(canvasPresets().value(0).width, canvasPresets().value(0).height));
}

int FormatBank::labelWidth() const {
    // The engraved caps' own width, tracking included.
    const QFont font = engravedFont(this->font(), kFormatLabel);
    return int(std::ceil(QFontMetricsF(font).horizontalAdvance(kFormatLabel.toUpper())));
}

QRectF FormatBank::bankRect() const {
    const int keys = int(m_keys.size());
    const int width = 2 * kBankPadding + keys * FormatKey::kWidth + std::max(0, keys - 1) * kKeyGap;
    return QRectF(labelWidth() + kLabelGap, 0, width, kHeight);
}

QSize FormatBank::sizeHint() const {
    return QSize(int(bankRect().right()), kHeight);
}

void FormatBank::layoutKeys() {
    const QRectF bank = bankRect();
    for (int i = 0; i < m_keys.size(); ++i)
        m_keys.at(i)->move(int(bank.left()) + kBankPadding + i * (FormatKey::kWidth + kKeyGap), kBankPadding);
}

void FormatBank::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    layoutKeys();
}

void FormatBank::changeEvent(QEvent *event) {
    QWidget::changeEvent(event);
    // The label's width follows the font the style sheet gives.
    if (event->type() == QEvent::FontChange || event->type() == QEvent::StyleChange) {
        setFixedWidth(sizeHint().width());
        layoutKeys();
        updateGeometry();
    }
}

void FormatBank::setCanvasSize(const QSize &size) {
    m_size = size;
    FormatKey *match = nullptr;
    for (FormatKey *key : std::as_const(m_keys))
        if (key->format() == size) match = key;
    if (match) {
        match->setChecked(true);
    } else {
        // An exclusive key cannot be unchecked while it is the checked one;
        // a custom size lets go of every key for a moment.
        for (FormatKey *key : std::as_const(m_keys)) {
            key->setAutoExclusive(false);
            key->setChecked(false);
            key->setAutoExclusive(true);
        }
    }
    const QString label = size.isValid() ? canvasSizeLabel(size) : QString();
    setToolTip(label);
    setAccessibleDescription(label);
}

int FormatBank::currentIndex() const {
    for (int i = 0; i < m_keys.size(); ++i)
        if (m_keys.at(i)->isChecked()) return i;
    return -1;
}

void FormatBank::choose(FormatKey *key) {
    if (key->format() == m_size) return;
    setCanvasSize(key->format());
    emit formatChosen(key->format());
}

void FormatBank::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    if (!isEnabled()) painter.setOpacity(0.5);
    painter.setFont(font());
    drawEngraved(painter, QRectF(0, 0, labelWidth() + 2, height()), Qt::AlignLeft | Qt::AlignVCenter, kFormatLabel);
    paintKeyBank(painter, bankRect());
}

// --------------------------------------------------------------- ExportButton

ExportButton::ExportButton(QAction *exportAction, QAction *renderAction, QWidget *parent)
    : QWidget(parent), m_exportAction(exportAction) {
    setObjectName(QStringLiteral("exportButton"));
    setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Fixed);
    setFixedHeight(kHeight);
    m_main = new ExportPart(this, [this] { showMenu(); });
    m_main->setObjectName(QStringLiteral("exportMain"));
    m_main->setAccessibleName(QStringLiteral("Export"));
    m_main->setAttribute(Qt::WA_Hover);
    m_arrow = new ExportPart(this, [this] { showMenu(); });
    m_arrow->setObjectName(QStringLiteral("exportOptions"));
    m_arrow->setAccessibleName(QStringLiteral("Export options"));
    m_arrow->setToolTip(QStringLiteral("Export quality, and Render Preview"));
    m_arrow->setAttribute(Qt::WA_Hover);

    m_menu = new QMenu(this);
    m_menu->setObjectName(QStringLiteral("exportMenu"));
    m_menu->setToolTipsVisible(true);
    m_qualities = new QActionGroup(this);
    m_qualities->setExclusive(true);
    // The finished file first and by default; a draft only when asked.
    m_master = m_menu->addAction(QStringLiteral("Master"));
    m_master->setData(QStringLiteral("full"));
    m_master->setToolTip(QStringLiteral("The project's own size and frame rate"));
    m_draft = m_menu->addAction(QStringLiteral("Draft\t360p · 15 fps"));
    m_draft->setData(QStringLiteral("preview"));
    m_draft->setToolTip(QStringLiteral("360p at 15 fps, quick to make and share for review"));
    for (QAction *choice : {m_master, m_draft}) {
        choice->setCheckable(true);
        m_qualities->addAction(choice);
    }
    m_master->setChecked(true);
    connect(m_qualities, &QActionGroup::triggered, this, [this] {
        syncFromAction();
        emit qualityChanged(quality());
    });
    if (renderAction) {
        m_menu->addSeparator();
        m_menu->addAction(renderAction);
    }
    connect(m_menu, &QMenu::aboutToShow, this, &ExportButton::menuAboutToShow);
    connect(m_menu, &QMenu::aboutToHide, this, [this] {
        m_menuOpen = false;
        update();
    });

    connect(m_main, &QAbstractButton::clicked, this, [this] {
        if (m_exportAction) m_exportAction->trigger();
    });
    connect(m_arrow, &QAbstractButton::pressed, this, &ExportButton::showMenu);
    if (exportAction) connect(exportAction, &QAction::changed, this, &ExportButton::syncFromAction);
    syncFromAction();
    setFixedWidth(sizeHint().width());
}

QString ExportButton::quality() const {
    return m_draft->isChecked() ? QStringLiteral("preview") : QStringLiteral("full");
}

QString ExportButton::qualityName() const {
    return m_draft->isChecked() ? QStringLiteral("Draft") : QStringLiteral("Master");
}

void ExportButton::setQuality(const QString &quality) {
    (quality == QStringLiteral("preview") ? m_draft : m_master)->setChecked(true);
    syncFromAction();
}

void ExportButton::setMasterDetail(const QString &detail) {
    m_master->setText(detail.isEmpty() ? QStringLiteral("Master") : QStringLiteral("Master\t%1").arg(detail));
}

QFont ExportButton::labelFont() const {
    QFont font = this->font();
    font.setPixelSize(12);
    font.setWeight(QFont::DemiBold);
    return font;
}

QFont ExportButton::chipFont() const {
    QFont font = this->font();
    font.setPixelSize(8);
    font.setWeight(QFont::Bold);
    font.setLetterSpacing(QFont::AbsoluteSpacing, 0.8);
    font.setCapitalization(QFont::AllUppercase);
    return font;
}

namespace {
constexpr qreal kLeadingPadding = 9;
constexpr qreal kChipGap = 6;
constexpr qreal kChipPadding = 4;
constexpr qreal kChipHeight = 12;
constexpr qreal kTrailingPadding = 7;
} // namespace

QRectF ExportButton::chipRect() const {
    const qreal label = QFontMetricsF(labelFont()).horizontalAdvance(QStringLiteral("Export"));
    // As wide for DRAFT as for MASTER, so choosing one moves nothing.
    const QFontMetricsF chip(chipFont());
    const qreal text = std::max(chip.horizontalAdvance(QStringLiteral("MASTER")), chip.horizontalAdvance(QStringLiteral("DRAFT")));
    return QRectF(std::round(kLeadingPadding + label + kChipGap), std::round((kHeight - kChipHeight) / 2),
                  std::ceil(text + 2 * kChipPadding), kChipHeight);
}

QSize ExportButton::sizeHint() const {
    return QSize(int(std::ceil(chipRect().right() + kTrailingPadding)) + kArrowWidth, kHeight);
}

void ExportButton::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    m_main->setGeometry(0, 0, width() - kArrowWidth, height());
    m_arrow->setGeometry(width() - kArrowWidth, 0, kArrowWidth, height());
}

void ExportButton::changeEvent(QEvent *event) {
    QWidget::changeEvent(event);
    if (event->type() == QEvent::FontChange) setFixedWidth(sizeHint().width());
}

void ExportButton::syncFromAction() {
    const bool enabled = !m_exportAction || m_exportAction->isEnabled();
    m_main->setEnabled(enabled);
    const QString what = m_draft->isChecked() ? QStringLiteral("Draft: 360p at 15 fps")
                                              : QStringLiteral("Master: the project's own size and frame rate");
    m_main->setAccessibleDescription(what);
    // The command's tooltip carries why it cannot run, when it cannot.
    const QString tip = m_exportAction ? m_exportAction->toolTip() : QStringLiteral("Export");
    m_main->setToolTip(QStringLiteral("%1\n%2").arg(tip, what));
    update();
}

void ExportButton::showMenu() {
    if (m_menu->isVisible()) return;
    // The menu takes the pointer's release, so the arrow is let up now and
    // shows held while its menu is open instead.
    m_arrow->setDown(false);
    m_menuOpen = true;
    update();
    m_menu->popup(mapToGlobal(QPoint(0, height() + 2)));
}

void ExportButton::paintEvent(QPaintEvent *) {
    const EditorTheme &t = editorTheme();
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    const qreal line = toolbarHairline(painter);
    const QRectF outer = toolbarSnap(painter, QRectF(rect()));
    const qreal radius = 5;
    QPainterPath body;
    body.addRoundedRect(outer, radius, radius);
    QPainterPath face;
    face.addRoundedRect(outer.adjusted(line, line, -line, -line), radius - line, radius - line);
    const bool enabled = m_main->isEnabled();
    const qreal split = std::round(width() - kArrowWidth);
    const QRectF mainRect(outer.left(), outer.top(), split - outer.left(), outer.height());
    const QRectF arrowRect(split, outer.top(), outer.right() - split, outer.height());

    // The body: the accent, lit from above, as the editor's primary
    // buttons; a disabled export is a disabled button's bezel.
    if (enabled) {
        painter.fillPath(body, t.accent);
        QLinearGradient lit(outer.topLeft(), outer.bottomLeft());
        lit.setColorAt(0, t.accentTop);
        lit.setColorAt(1, t.accent);
        painter.fillPath(face, lit);
    } else {
        painter.fillPath(body, t.buttonDisabledEdge);
        painter.fillPath(face, t.buttonDisabled);
    }
    // Each part's own wash: lighter under the pointer, darker held (the
    // arrow stays held while its menu is open).
    const auto wash = [&](const QRectF &part, bool hovered, bool held) {
        if (!hovered && !held) return;
        QPainterPath area;
        area.addRect(part);
        const QColor fill = enabled ? (held ? t.accentPressed : t.accentTop) : (held ? t.buttonPressed : t.buttonHover);
        painter.fillPath(face.intersected(area), fill);
    };
    wash(mainRect, m_main->underMouse() && enabled, m_main->isDown());
    wash(arrowRect, m_arrow->underMouse(), m_arrow->isDown() || m_menuOpen);

    const QColor ink = enabled ? t.onAccent : t.buttonDisabledText;
    // The rule between the parts.
    painter.fillRect(toolbarSnap(painter, QRectF(split, outer.top() + 4, line, outer.height() - 8)), withAlpha(ink, 0.35));

    painter.setFont(labelFont());
    painter.setPen(ink);
    painter.drawText(QRectF(kLeadingPadding, 0, chipRect().left() - kLeadingPadding, height()), Qt::AlignLeft | Qt::AlignVCenter,
                     QStringLiteral("Export"));
    // The chip: the quality, outlined in the key's own ink.
    const QRectF chip = toolbarSnap(painter, chipRect());
    QPainterPath chipEdge;
    chipEdge.addRoundedRect(chip, 3, 3);
    QPainterPath chipInside;
    chipInside.addRoundedRect(chip.adjusted(line, line, -line, -line), 3 - line, 3 - line);
    painter.fillPath(chipEdge.subtracted(chipInside), withAlpha(ink, 0.6));
    painter.setFont(chipFont());
    painter.drawText(chip, Qt::AlignCenter, qualityName().toUpper());

    // The arrow: a small chevron, in the ink whenever the menu can open.
    const QColor arrowInk = enabled ? t.onAccent : (m_arrow->isEnabled() ? t.buttonText : t.buttonDisabledText);
    const QPointF centre = arrowRect.center();
    QPainterPath chevron;
    chevron.moveTo(centre + QPointF(-3.5, -1.5));
    chevron.lineTo(centre + QPointF(0, 2));
    chevron.lineTo(centre + QPointF(3.5, -1.5));
    painter.setPen(QPen(arrowInk, 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    painter.setBrush(Qt::NoBrush);
    painter.drawPath(chevron);
}

} // namespace kerf
