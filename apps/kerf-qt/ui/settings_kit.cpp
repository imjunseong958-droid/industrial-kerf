#include "settings_kit.h"

#include "design_tokens.h"

#include <QHBoxLayout>
#include <QLabel>
#include <QPaintEvent>
#include <QPainter>
#include <QVBoxLayout>
#include <QVariantAnimation>

namespace kerf::ui {

using namespace tokens;

namespace {

constexpr int kTrackWidth = 32;
constexpr int kTrackHeight = 18;
constexpr int kKnob = 14;
constexpr int kKnobInset = (kTrackHeight - kKnob) / 2;
// A row's air above and below its text, and between its text and its control.
constexpr int kRowPad = space::s12;
constexpr int kRowGap = 24;
// What a row says of itself wraps before its lines grow too long to read.
constexpr int kTextMost = 600;
// The page's edges, and from its name to its first row.
constexpr int kPageSide = 24;
constexpr int kPageTop = 20;
constexpr int kGroupGap = space::s16;

QString rgba(QRgb color) {
    return QStringLiteral("rgba(%1, %2, %3, %4)").arg(qRed(color)).arg(qGreen(color)).arg(qBlue(color)).arg(qAlpha(color));
}

} // namespace

// ================================================================ Switch

Switch::Switch(QWidget *parent) : KitButton(parent) {
    setCheckable(true);
    setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Fixed);
    setFocusPolicy(Qt::TabFocus);
    setCursor(Qt::ArrowCursor);
    m_slide = new QVariantAnimation(this);
    m_slide->setEasingCurve(easing());
    connect(m_slide, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) {
        m_position = value.toReal();
        update();
    });
    connect(this, &QAbstractButton::toggled, this, [this](bool on) {
        const qreal target = on ? 1.0 : 0.0;
        m_slide->stop();
        const int duration = isVisible() ? motionDuration(motion::segment) : 0;
        if (duration == 0) {
            m_position = target;
            update();
            return;
        }
        m_slide->setDuration(duration);
        m_slide->setStartValue(m_position);
        m_slide->setEndValue(target);
        m_slide->start();
    });
}

QSize Switch::sizeHint() const { return {kTrackWidth, kTrackHeight}; }

void Switch::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    if (!isEnabled()) p.setOpacity(0.5);
    const QRectF groove((width() - kTrackWidth) / 2.0, (height() - kTrackHeight) / 2.0, kTrackWidth, kTrackHeight);
    const QColor off = mix(color(track), color(track2), 0.35 * hoverProgress());
    const QColor lit = mix(color(accent), color(accentHover), hoverProgress());
    p.setPen(Qt::NoPen);
    p.setBrush(mix(off, lit, m_position));
    p.drawRoundedRect(groove, kTrackHeight / 2.0, kTrackHeight / 2.0);
    const qreal travel = kTrackWidth - 2 * kKnobInset - kKnob;
    p.setBrush(mix(color(ink2), color(accentText), m_position));
    p.drawEllipse(QRectF(groove.left() + kKnobInset + m_position * travel, groove.top() + kKnobInset, kKnob, kKnob));
    if (focusVisible()) {
        p.setOpacity(1);
        paintFocusRing(p, groove, kTrackHeight / 2.0);
    }
}

// ================================================================ SettingsRow

SettingsRow::SettingsRow(const QString &label, const QString &description, QWidget *control, QWidget *parent)
    : QWidget(parent), m_control(control) {
    setObjectName(QStringLiteral("settingsRow"));
    m_column = new QVBoxLayout(this);
    m_column->setContentsMargins(0, kRowPad, 0, kRowPad);
    m_column->setSpacing(space::s8);
    auto *line = new QHBoxLayout;
    line->setContentsMargins(0, 0, 0, 0);
    line->setSpacing(kRowGap);
    // What the row says keeps to a readable width, from the row's start; the
    // rest of the row is air, and its control stands at the end.
    auto *words = new QWidget;
    words->setMaximumWidth(kTextMost);
    auto *text = new QVBoxLayout(words);
    text->setContentsMargins(0, 0, 0, 0);
    text->setSpacing(space::s2);
    m_label = new QLabel(label);
    m_label->setObjectName(QStringLiteral("settingsRowLabel"));
    text->addWidget(m_label);
    auto *said = new WrapLabel;
    said->setObjectName(QStringLiteral("settingsRowHint"));
    said->setWordWrap(true);
    said->setTextFormat(Qt::PlainText);
    m_description = said;
    text->addWidget(m_description);
    line->addWidget(words, 1000);
    line->addStretch(1);
    if (control) {
        line->addWidget(control, 0, Qt::AlignRight | Qt::AlignVCenter);
        if (control->accessibleName().isEmpty()) control->setAccessibleName(label);
    }
    m_column->addLayout(line);
    setDescription(description);
}

QString SettingsRow::label() const { return m_label->text(); }

QString SettingsRow::description() const { return m_description->text(); }

void SettingsRow::setDescription(const QString &description) {
    m_description->setText(description);
    m_description->setVisible(!description.isEmpty());
    if (m_control) m_control->setAccessibleDescription(description);
}

void SettingsRow::addBelow(QWidget *widget) { m_column->addWidget(widget); }

void SettingsRow::addBelow(QLayout *layout) { m_column->addLayout(layout); }

void SettingsRow::setLineAbove(bool line) {
    if (line == m_lineAbove) return;
    m_lineAbove = line;
    update();
}

void SettingsRow::paintEvent(QPaintEvent *) {
    if (!m_lineAbove) return;
    QPainter p(this);
    p.fillRect(QRect(0, 0, width(), layout::seam), color(line));
}

// ================================================================ SettingsGroup

SettingsGroup::SettingsGroup(const QString &label, QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("settingsGroup"));
    m_column = new QVBoxLayout(this);
    m_column->setContentsMargins(0, 0, 0, 0);
    m_column->setSpacing(0);
    if (label.isEmpty()) return;
    auto *name = new QLabel(label);
    name->setObjectName(QStringLiteral("settingsSection"));
    m_column->addWidget(name);
    m_column->addSpacing(space::s4);
    setAccessibleName(label);
}

SettingsRow *SettingsGroup::addRow(const QString &label, const QString &description, QWidget *control) {
    auto *row = new SettingsRow(label, description, control);
    row->setLineAbove(m_continues || !m_rows.isEmpty());
    m_rows << row;
    m_column->addWidget(row);
    return row;
}

SettingsGroup *SettingsGroup::addRows() {
    auto *rows = new SettingsGroup;
    rows->m_continues = m_continues || !m_rows.isEmpty();
    m_column->addWidget(rows);
    return rows;
}

// ================================================================ SettingsPage

SettingsPage::SettingsPage(const QString &title, QWidget *parent) : QScrollArea(parent) {
    setWidgetResizable(true);
    setFrameShape(QFrame::NoFrame);
    setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    viewport()->setAutoFillBackground(false);
    auto *content = new QWidget;
    content->setObjectName(QStringLiteral("settingsPageContent"));
    content->setAttribute(Qt::WA_TranslucentBackground);
    m_column = new QVBoxLayout(content);
    m_column->setContentsMargins(kPageSide, kPageTop, kPageSide, kPageSide);
    m_column->setSpacing(0);
    m_title = new QLabel(title);
    m_title->setObjectName(QStringLiteral("settingsPageTitle"));
    m_column->addWidget(m_title);
    m_column->addSpacing(space::s4);
    m_column->addStretch(1);
    setWidget(content);
    setAccessibleName(title);
    setStyleSheet(QStringLiteral("QScrollArea { background: transparent; border: 0; }"
                                 "QLabel#settingsPageTitle { color: %1; font-size: 18px; font-weight: 500; }"
                                 "QLabel#settingsSection { color: %2; font-size: 13px; }"
                                 "QLabel#settingsRowLabel { color: %1; font-size: 14px; }"
                                 "QLabel#settingsRowHint { color: %2; font-size: 13px; }"
                                 "QLabel[quiet=\"true\"] { color: %2; font-size: 13px; }")
                      .arg(rgba(ink), rgba(ink3)));
}

QString SettingsPage::title() const { return m_title->text(); }

SettingsGroup *SettingsPage::addGroup(const QString &label) {
    auto *group = new SettingsGroup(label);
    // Before the stretch at the page's foot; a named group stands apart from
    // the rows over it.
    const int at = m_column->count() - 1;
    if (!label.isEmpty()) {
        m_column->insertSpacing(at, kGroupGap);
        m_column->insertWidget(at + 1, group);
    } else {
        m_column->insertWidget(at, group);
    }
    return group;
}

} // namespace kerf::ui
