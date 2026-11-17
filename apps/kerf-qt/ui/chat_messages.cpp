#include "chat_messages.h"
#include "make_card.h"

#include "chat_parts.h"
#include "components.h"
#include "design_tokens.h"

#include <QAbstractButton>
#include <QClipboard>
#include <QGuiApplication>
#include <QFontMetricsF>
#include <QHBoxLayout>
#include <QLabel>
#include <QLinearGradient>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QTimer>
#include <QVBoxLayout>

#include <algorithm>
#include <cmath>
#include <functional>

namespace kerf {

using namespace tokens;

namespace {

void setInk(QWidget *widget, QRgb ink) {
    QPalette palette = widget->palette();
    palette.setColor(QPalette::WindowText, color(ink));
    palette.setColor(QPalette::Text, color(ink));
    widget->setPalette(palette);
}

// Follows a tc:<seconds> link of richBody's to a seek.
void followTimecodes(QLabel *label, QObject *context, const std::function<void(double)> &seek) {
    QObject::connect(label, &QLabel::linkActivated, context, [seek](const QString &link) {
        if (!link.startsWith(QLatin1String("tc:"))) return;
        bool ok = false;
        const double seconds = link.mid(3).toDouble(&ok);
        if (ok && std::isfinite(seconds)) seek(seconds);
    });
}

// The timecode a request was asked at: mono 11 in accent ink on a raised
// chip, radius 4; it seeks.
class TimecodeChip final : public QAbstractButton {
public:
    TimecodeChip(const QString &text, QWidget *parent) : QAbstractButton(parent) {
        setText(text);
        setCursor(Qt::PointingHandCursor);
        setFocusPolicy(Qt::TabFocus);
        setFont(tokens::font(monoSmall));
        ui::setTip(this, QStringLiteral("Go to %1").arg(text));
        setAccessibleName(QStringLiteral("Asked at %1").arg(text));
    }
    QSize sizeHint() const override {
        const QFontMetricsF metrics(font());
        return QSize(int(std::ceil(metrics.horizontalAdvance(text()))) + 2 * space::s6, height::h20);
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing);
        const bool hover = underMouse() || isDown();
        p.setPen(Qt::NoPen);
        p.setBrush(color(hover ? hover2 : raised));
        p.drawRoundedRect(QRectF(rect()), radius::small, radius::small);
        p.setPen(color(accentInk));
        p.setFont(font());
        p.drawText(rect(), Qt::AlignCenter, text());
        if (hasFocus()) ui::paintFocusRing(p, QRectF(rect()), radius::small);
    }
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }
};

} // namespace

QString workClock(double seconds) {
    const qint64 whole = std::max<qint64>(0, qint64(std::floor(seconds)));
    return QStringLiteral("%1:%2").arg(whole / 60).arg(whole % 60, 2, 10, QLatin1Char('0'));
}

QString workLength(double seconds) {
    const qint64 whole = std::max<qint64>(1, qint64(std::llround(seconds)));
    if (whole < 60) return QStringLiteral("%1s").arg(whole);
    if (whole < 3600) return QStringLiteral("%1m %2s").arg(whole / 60).arg(whole % 60);
    return QStringLiteral("%1h %2m").arg(whole / 3600).arg((whole % 3600) / 60);
}

// ------------------------------------------------------------------ YouMessage

YouMessage::YouMessage(const QString &text, double timecode, double fps, QWidget *parent)
    : QWidget(parent), m_text(text.trimmed()), m_timecode(timecode) {
    setObjectName(QStringLiteral("youMessage"));
    setAccessibleName(QStringLiteral("You"));
    m_body = new QLabel(this);
    m_body->setObjectName(QStringLiteral("youText"));
    m_body->setFont(tokens::font(reading));
    m_body->setWordWrap(true);
    // Rich text only so that a long word (a file's name, a link) breaks.
    m_body->setTextFormat(Qt::RichText);
    m_body->setTextInteractionFlags(Qt::TextSelectableByMouse);
    m_body->setText(chat::breakingLongWords(m_text).replace(QLatin1Char('\n'), QStringLiteral("<br>")));
    setInk(m_body, ink);
    if (std::isfinite(timecode)) {
        auto *chip = new TimecodeChip(chat::timecode(timecode, fps), this);
        connect(chip, &QAbstractButton::clicked, this, [this] { emit seekRequested(m_timecode); });
        m_chip = chip;
    }
    setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Preferred);
}

int YouMessage::chipHeight() const { return m_chip ? height::h20 + space::s4 : 0; }

QRect YouMessage::bubbleFor(int width) const {
    const int most = std::max(80, int(width * kShare));
    const QFontMetricsF metrics(m_body->font());
    const int textMost = most - 2 * kPadX;
    const QRectF ideal = metrics.boundingRect(QRectF(0, 0, textMost, 1e6), Qt::TextWordWrap, m_text);
    const int textWidth = std::min(textMost, int(std::ceil(ideal.width())) + 1);
    const int textHeight = std::max(reading.line, m_body->heightForWidth(textWidth));
    const int bubbleWidth = textWidth + 2 * kPadX;
    return QRect(width - bubbleWidth, chipHeight(), bubbleWidth, textHeight + 2 * kPadY);
}

int YouMessage::heightForWidth(int width) const { return bubbleFor(width).bottom() + 1; }

QSize YouMessage::sizeHint() const {
    const int width = this->width() > 0 ? this->width() : 400;
    return QSize(width, heightForWidth(width));
}

void YouMessage::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    const QRect bubble = bubbleFor(width());
    m_body->setGeometry(bubble.adjusted(kPadX, kPadY, -kPadX, -kPadY));
    if (m_chip) {
        const QSize chip = m_chip->sizeHint();
        m_chip->setGeometry(width() - chip.width(), 0, chip.width(), chip.height());
    }
}

void YouMessage::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    const QRectF bubble = QRectF(bubbleFor(width())).adjusted(0.5, 0.5, -0.5, -0.5);
    p.setPen(QPen(color(line), 1));
    p.setBrush(color(raised));
    p.drawRoundedRect(bubble, radius::card, radius::card);
}

// ------------------------------------------------------------------ WorkLog

WorkLog::WorkLog(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("workLog"));
    setAccessibleName(QStringLiteral("Work log"));
    m_toggle = new ChatExpander(this);
    m_toggle->setObjectName(QStringLiteral("workLogToggle"));
    m_toggle->setCheckable(true);
    m_toggle->hide();
    connect(m_toggle, &QAbstractButton::toggled, this, [this](bool open) {
        m_open = open;
        updateGeometry();
        update();
    });
    m_frame = new QTimer(this);
    // The light goes over "Working" once in a second and a half, drawn at 30 a second.
    m_frame->setInterval(33);
    connect(m_frame, &QTimer::timeout, this, [this] {
        if (!m_live) return;
        m_shimmer = std::fmod(m_elapsed.elapsed() / double(motion::workShimmer), 1.0);
        update();
    });
    setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Preferred);
    hide();
}

void WorkLog::start() {
    m_live = true;
    m_open = true;
    m_elapsed.start();
    m_toggle->hide();
    show();
    updateTimer();
    updateGeometry();
    update();
}

void WorkLog::setStatus(const QString &status) {
    const QString line = status.simplified();
    if (line == m_status) return;
    m_status = line;
    update();
}

void WorkLog::addStep(const QString &step) {
    const QString line = step.simplified();
    if (line.isEmpty() || (!m_steps.isEmpty() && m_steps.constLast() == line)) return;
    m_steps << line;
    updateToggle();
    updateGeometry();
    update();
}

void WorkLog::setStep(const QString &id, const QString &step, bool failed) {
    const QString line = step.simplified();
    if (line.isEmpty()) return;
    if (id.isEmpty()) {
        addStep(line);
        return;
    }
    if (failed && !m_failedIds.contains(id)) m_failedIds << id;
    while (m_stepIds.size() < m_steps.size()) m_stepIds << QString();
    const qsizetype at = m_stepIds.indexOf(id);
    if (at >= 0) {
        m_steps[at] = line;
    } else {
        m_steps << line;
        m_stepIds << id;
    }
    updateToggle();
    updateGeometry();
    update();
}

void WorkLog::setSteps(const QStringList &steps) {
    m_stepIds.clear();
    m_failedIds.clear();
    m_steps.clear();
    for (const QString &step : steps)
        if (!step.simplified().isEmpty()) m_steps << step.simplified();
    updateToggle();
    updateGeometry();
    update();
}

void WorkLog::finish(double seconds) {
    if (m_live) m_seconds = seconds > 0 ? seconds : m_elapsed.elapsed() / 1000.0;
    else m_seconds = seconds;
    m_live = false;
    m_status.clear();
    m_open = false;
    updateTimer();
    updateToggle();
    setVisible(true);
    updateGeometry();
    update();
}

double WorkLog::seconds() const { return m_live ? m_elapsed.elapsed() / 1000.0 : m_seconds; }

QString WorkLog::headline() const {
    if (m_live) return QStringLiteral("Working · %1").arg(workClock(seconds()));
    QString line = QStringLiteral("Worked for %1").arg(workLength(m_seconds));
    if (!m_steps.isEmpty()) line += m_steps.size() == 1 ? QStringLiteral(" · 1 step") : QStringLiteral(" · %1 steps").arg(m_steps.size());
    return line;
}

void WorkLog::setOpen(bool open) {
    if (m_live) return;
    m_toggle->setChecked(open);
}

void WorkLog::updateToggle() {
    m_toggle->setText(headline());
    m_toggle->setEnabled(!m_steps.isEmpty());
    m_toggle->setChevron(!m_steps.isEmpty());
    m_toggle->setVisible(!m_live);
    m_toggle->setOpenQuietly(m_open);
    m_toggle->adjustSize();
    m_toggle->move(0, 0);
}

void WorkLog::updateTimer() {
    const bool run = m_live && isVisible() && !ui::reducedMotion();
    if (run && !m_frame->isActive()) m_frame->start();
    if (!run) m_frame->stop();
    // The clock goes on with no light: once a second.
    if (m_live && ui::reducedMotion()) {
        m_frame->setInterval(1000);
        m_frame->start();
    } else {
        m_frame->setInterval(33);
    }
}

void WorkLog::showEvent(QShowEvent *event) {
    QWidget::showEvent(event);
    updateTimer();
}

void WorkLog::hideEvent(QHideEvent *event) {
    QWidget::hideEvent(event);
    m_frame->stop();
}

QStringList WorkLog::shownSteps() const {
    if (m_live) return m_steps.mid(std::max<qsizetype>(0, m_steps.size() - kLiveSteps));
    return m_open ? m_steps : QStringList{};
}

int WorkLog::heightForWidth(int) const {
    const qsizetype steps = shownSteps().size();
    return kRow + (steps > 0 ? space::s4 + int(steps) * kRow : 0);
}

QSize WorkLog::sizeHint() const {
    const int width = this->width() > 0 ? this->width() : 320;
    return QSize(width, heightForWidth(width));
}

void WorkLog::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    if (m_live) {
        // "Working": ink3, with a band of ink passing over it.
        const QFont word = tokens::font(controlStrong);
        const QFontMetricsF metrics(word);
        const QString label = QStringLiteral("Working");
        const qreal labelWidth = metrics.horizontalAdvance(label);
        const qreal baseline = (kRow - metrics.height()) / 2 + metrics.ascent();
        QPen pen(color(ink2));
        if (!ui::reducedMotion()) {
            const qreal band = labelWidth * 0.6;
            const qreal centre = -band + m_shimmer * (labelWidth + 2 * band);
            QLinearGradient light(centre - band, 0, centre + band, 0);
            light.setColorAt(0.0, color(ink3));
            light.setColorAt(0.5, color(ink));
            light.setColorAt(1.0, color(ink3));
            pen = QPen(QBrush(light), 1);
        }
        p.setFont(word);
        p.setPen(pen);
        p.drawText(QPointF(0, baseline), label);
        // The time so far, then the step it is on.
        const QFont clock = tokens::font(monoSmall);
        const QFontMetricsF clockMetrics(clock);
        const QString time = QStringLiteral(" · %1").arg(workClock(seconds()));
        p.setFont(clock);
        p.setPen(color(ink3));
        qreal x = labelWidth;
        p.drawText(QPointF(x, (kRow - clockMetrics.height()) / 2 + clockMetrics.ascent()), time);
        x += clockMetrics.horizontalAdvance(time) + space::s8;
        if (!m_status.isEmpty() && x < width()) {
            const QFont status = tokens::font(control);
            const QFontMetricsF statusMetrics(status);
            p.setFont(status);
            p.drawText(QPointF(x, (kRow - statusMetrics.height()) / 2 + statusMetrics.ascent()),
                       statusMetrics.elidedText(m_status, Qt::ElideRight, width() - x));
        }
    }
    // The steps, on a hairline: a dot each, ink2 (the latest ink while live).
    const QStringList steps = shownSteps();
    if (steps.isEmpty()) return;
    // The rows shown are the last of m_steps.
    const qsizetype first = m_steps.size() - steps.size();
    const QFont font = tokens::font(control);
    const QFontMetricsF metrics(font);
    p.setFont(font);
    const qreal top = kRow + space::s4;
    const qreal rule = 3.5;
    p.fillRect(QRectF(rule - 0.5, top + 4, 1, steps.size() * kRow - 8), color(line2));
    for (qsizetype index = 0; index < steps.size(); ++index) {
        const qreal y = top + index * kRow;
        const bool latest = m_live && index == steps.size() - 1;
        p.setPen(Qt::NoPen);
        p.setBrush(color(latest ? ink2 : track2));
        p.drawEllipse(QPointF(rule, y + kRow / 2.0), 2.5, 2.5);
        const qsizetype row = first + index;
        const bool failed = row < m_stepIds.size() && !m_stepIds.at(row).isEmpty() && m_failedIds.contains(m_stepIds.at(row));
        p.setPen(color(failed ? neg : latest ? ink : ink2));
        const qreal x = space::s16;
        p.drawText(QPointF(x, y + (kRow - metrics.height()) / 2 + metrics.ascent()),
                   metrics.elidedText(steps.at(index), Qt::ElideRight, width() - x));
    }
}

// ------------------------------------------------------------------ KerfMessage

KerfMessage::KerfMessage(double fps, QWidget *parent) : QWidget(parent), m_fps(fps) {
    setObjectName(QStringLiteral("kerfMessage"));
    setAccessibleName(QStringLiteral("Kerf"));
    m_layout = new QVBoxLayout(this);
    m_layout->setContentsMargins(0, 0, 0, 0);
    m_layout->setSpacing(space::s8);

    m_work = new WorkLog;
    m_layout->addWidget(m_work);

    m_state = new QLabel;
    m_state->setObjectName(QStringLiteral("kerfState"));
    m_state->setFont(tokens::font(controlStrong));
    m_state->hide();
    m_layout->addWidget(m_state);

    m_dots = new DraftDots;
    m_dots->hide();
    m_layout->addWidget(m_dots);

    m_bodyLabel = new ui::WrapLabel;
    m_bodyLabel->setObjectName(QStringLiteral("kerfText"));
    m_bodyLabel->setFont(tokens::font(reading));
    m_bodyLabel->setWordWrap(true);
    m_bodyLabel->setTextFormat(Qt::RichText);
    m_bodyLabel->setTextInteractionFlags(Qt::TextBrowserInteraction);
    m_bodyLabel->setOpenExternalLinks(false);
    m_bodyLabel->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    setInk(m_bodyLabel, ink);
    m_bodyLabel->hide();
    followTimecodes(m_bodyLabel, this, [this](double seconds) { emit seekRequested(seconds); });
    m_layout->addWidget(m_bodyLabel);

    m_cardBox = new QWidget;
    m_cardLayout = new QVBoxLayout(m_cardBox);
    m_cardLayout->setContentsMargins(0, space::s4, 0, 0);
    m_cardLayout->setSpacing(space::s8);
    m_cardBox->hide();
    m_layout->addWidget(m_cardBox);

    // The makings it asked for, kept apart from the proposal cards, which
    // are made again as proposals change.
    m_askBox = new QWidget;
    m_askLayout = new QVBoxLayout(m_askBox);
    m_askLayout->setContentsMargins(0, space::s4, 0, 0);
    m_askLayout->setSpacing(space::s8);
    m_askBox->hide();
    m_layout->addWidget(m_askBox);

    m_askAgain = new ChatExpander;
    m_askAgain->setObjectName(QStringLiteral("askAgain"));
    m_askAgain->setText(QStringLiteral("Ask again"));
    m_askAgain->setChevron(false);
    m_askAgain->hide();
    connect(m_askAgain, &QAbstractButton::clicked, this, [this] { emit askAgain(m_request); });
    // Under a finished answer: Copy, its text as written (Markdown and all),
    // and Ask again where it failed or was stopped.
    m_copy = new ChatExpander;
    m_copy->setObjectName(QStringLiteral("copyAnswer"));
    m_copy->setText(QStringLiteral("Copy"));
    m_copy->setChevron(false);
    m_copy->setCheckable(false);
    m_copy->hide();
    connect(m_copy, &QAbstractButton::clicked, this, [this] {
        QGuiApplication::clipboard()->setText(m_text.trimmed());
        m_copy->setText(QStringLiteral("Copied"));
        QTimer::singleShot(1500, m_copy, [copy = m_copy] { copy->setText(QStringLiteral("Copy")); });
    });
    auto *actions = new QHBoxLayout;
    actions->setContentsMargins(0, 0, 0, 0);
    actions->setSpacing(space::s12);
    actions->addWidget(m_copy);
    actions->addWidget(m_askAgain);
    actions->addStretch(1);
    m_layout->addLayout(actions);
}

void KerfMessage::setText(const QString &text) {
    m_text = text;
    updateBody();
}

void KerfMessage::appendText(const QString &delta) {
    m_text += delta;
    updateBody();
}

void KerfMessage::updateBody() {
    const bool has = !m_text.trimmed().isEmpty();
    if (has) m_bodyLabel->setText(chat::richBody(m_text.trimmed(), m_fps));
    m_bodyLabel->setVisible(has);
    if (has) m_dots->hide();
}

void KerfMessage::setDrafting(bool drafting) {
    m_dots->setVisible(drafting && m_text.trimmed().isEmpty());
    // Copy once there is a whole answer to copy.
    m_copy->setVisible(!drafting && !m_work->isLive() && !m_text.trimmed().isEmpty());
}

bool KerfMessage::isDrafting() const { return !m_dots->isHidden(); }

void KerfMessage::setOutcome(Outcome outcome, const QString &askAgain) {
    m_outcome = outcome;
    m_request = askAgain;
    m_dots->hide();
    switch (outcome) {
    case Outcome::Answer:
        m_state->hide();
        setInk(m_bodyLabel, ink);
        break;
    case Outcome::Failed:
        m_state->setText(QStringLiteral("Couldn't finish"));
        setInk(m_state, neg);
        m_state->show();
        setInk(m_bodyLabel, ink2);
        break;
    case Outcome::Stopped:
        m_state->setText(QStringLiteral("Stopped"));
        setInk(m_state, ink3);
        m_state->show();
        setInk(m_bodyLabel, ink2);
        break;
    }
    m_askAgain->setVisible(outcome != Outcome::Answer && !askAgain.trimmed().isEmpty());
    m_copy->setVisible(!m_work->isLive() && !m_text.trimmed().isEmpty());
}

void KerfMessage::addCard(QWidget *card) {
    m_cardLayout->addWidget(card);
    m_cardBox->show();
}

void KerfMessage::addAsk(MakeCard *card) {
    for (MakeCard *shown : asks())
        if (shown->ask().key() == card->ask().key()) {
            card->deleteLater();
            return;
        }
    m_askLayout->addWidget(card);
    m_askBox->show();
}

QList<MakeCard *> KerfMessage::asks() const {
    QList<MakeCard *> out;
    for (int index = 0; index < m_askLayout->count(); ++index)
        if (auto *card = qobject_cast<MakeCard *>(m_askLayout->itemAt(index)->widget())) out << card;
    return out;
}

QList<QWidget *> KerfMessage::cards() const {
    QList<QWidget *> out;
    for (int index = 0; index < m_cardLayout->count(); ++index)
        if (QWidget *widget = m_cardLayout->itemAt(index)->widget()) out << widget;
    return out;
}

void KerfMessage::clearCards() {
    while (QLayoutItem *item = m_cardLayout->takeAt(0)) {
        if (QWidget *widget = item->widget()) {
            widget->hide();
            widget->deleteLater();
        }
        delete item;
    }
    m_cardBox->hide();
}

} // namespace kerf
