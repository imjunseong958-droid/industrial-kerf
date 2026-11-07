#include "qc_view.h"
#include "components.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "tray_shell.h"

#include <QAction>
#include <QEnterEvent>
#include <QFontMetrics>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QLabel>
#include <QPainter>
#include <QToolButton>
#include <QVBoxLayout>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

constexpr int kRowHeight = tokens::height::h28;
constexpr int kPad = tokens::space::s12;
constexpr int kActionWidth = 64;
constexpr int kDot = 6;

// "In thread": UI 12 ink2, accent-ink on hover, right-aligned in its 64.
class ThreadLink final : public QToolButton {
public:
    explicit ThreadLink(QWidget *parent) : QToolButton(parent) {
        setObjectName(QStringLiteral("qcInThread"));
        setText(QStringLiteral("In thread"));
        setCursor(Qt::PointingHandCursor);
        setFixedSize(ui::snapWidth(tokens::control, text()) + tokens::space::s2, kRowHeight);
        setAttribute(Qt::WA_Hover);
        ui::setTip(this, QStringLiteral("Open the thread this finding is in"));
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setFont(tokens::font(tokens::control));
        p.setPen(tokens::color(underMouse() ? tokens::accentInk : tokens::ink2));
        p.drawText(rect(), Qt::AlignRight | Qt::AlignVCenter, text());
        if (hasFocus()) ui::paintFocusRing(p, QRectF(rect()).adjusted(2, 4, 0, -4), tokens::radius::control);
    }
};

QString rowValueText(const QJsonObject &row) {
    const QString measured = row.value(QStringLiteral("value")).toVariant().toString();
    const QString limit = row.value(QStringLiteral("limit")).toVariant().toString();
    if (limit.isEmpty()) return measured;
    if (measured.isEmpty()) return limit;
    return QStringLiteral("%1 · %2").arg(measured, limit);
}

// As much of a row's value as fits, each step saying less and none of them cutting a word, since a
// number cut short reads as another number: all of it, the measure before its limit, or the measure's
// first word, which is its number. Nothing when not even that fits; the tip has it all.
QString valueThatFits(const QString &value, const tokens::TypeStyle &style, double room) {
    const auto fits = [&style, room](const QString &text) { return ui::snapWidth(style, text) <= room; };
    if (fits(value)) return value;
    const QString measure = value.section(QStringLiteral(" · "), 0, 0);
    if (fits(measure)) return measure;
    const QString number = measure.section(QLatin1Char(' '), 0, 0);
    return fits(number) ? number : QString();
}

} // namespace

// ================================================================ QcRow

QcRow::QcRow(const QJsonObject &row, QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("qcRow"));
    m_name = row.value(QStringLiteral("name")).toString(row.value(QStringLiteral("code")).toString());
    m_value = rowValueText(row);
    m_passed = row.value(QStringLiteral("passed")).toBool();
    m_thread = row.value(QStringLiteral("commentId")).toString();
    setFixedHeight(kRowHeight);
    setAttribute(Qt::WA_Hover);
    ui::setTip(this, m_value.isEmpty() ? m_name : QStringLiteral("%1: %2").arg(m_name, m_value));
    if (!m_passed && !m_thread.isEmpty()) {
        auto *link = new ThreadLink(this);
        auto *layout = new QHBoxLayout(this);
        layout->setContentsMargins(0, 0, kPad, 0);
        layout->addStretch(1);
        layout->addWidget(link);
        connect(link, &QToolButton::clicked, this, [this] { emit threadRequested(m_thread); });
    }
}

QColor QcRow::valueColor() const {
    return tokens::color(m_passed ? tokens::ink2 : tokens::neg);
}

QSize QcRow::sizeHint() const {
    return QSize(240, kRowHeight);
}

void QcRow::enterEvent(QEnterEvent *) {
    m_hovered = true;
    update();
}

void QcRow::leaveEvent(QEvent *) {
    m_hovered = false;
    update();
}

void QcRow::paintEvent(QPaintEvent *) {
    QPainter p(this);
    if (m_hovered) p.fillRect(rect(), tokens::color(tokens::raised));
    p.setRenderHint(QPainter::Antialiasing, true);
    p.setPen(Qt::NoPen);
    p.setBrush(tokens::color(m_passed ? tokens::pos : tokens::neg));
    p.drawEllipse(QRectF(kPad, (height() - kDot) / 2.0, kDot, kDot));
    p.setRenderHint(QPainter::Antialiasing, false);

    const QString outcome = m_thread.isEmpty() || m_passed ? (m_passed ? QStringLiteral("Pass") : QStringLiteral("Fail")) : QString();
    const QRectF action(width() - kPad - kActionWidth, 0, kActionWidth, height());
    if (!outcome.isEmpty()) {
        // A failing row no comment carries has nothing to open.
        p.setFont(tokens::font(tokens::control));
        p.setPen(tokens::color(m_passed ? tokens::ink3 : tokens::neg));
        p.drawText(action, Qt::AlignRight | Qt::AlignVCenter, outcome);
    }
    const double nameLeft = kPad + kDot + tokens::space::s8;
    const double nameEnd = nameLeft + ui::snapWidth(tokens::control, m_name);
    // The values line up at the column an In thread needs, and one that does not fit there has what
    // this row's own action leaves. The name reads first, since it says what the row is.
    const double ownAction = ui::snapWidth(tokens::control, outcome.isEmpty() ? QStringLiteral("In thread") : outcome);
    const double column = action.left() - tokens::space::s8;
    const double furthest = width() - kPad - ownAction - tokens::space::s8;
    const bool inColumn = ui::snapWidth(tokens::monoSmall, m_value) <= column - nameEnd - tokens::space::s8;
    const double valueRight = inColumn ? column : furthest;
    const QString value = valueThatFits(m_value, tokens::monoSmall, valueRight - nameEnd - tokens::space::s8);
    p.setFont(tokens::font(tokens::monoSmall));
    p.setPen(valueColor());
    p.drawText(QRectF(nameEnd, 0, valueRight - nameEnd, height()), Qt::AlignRight | Qt::AlignVCenter, value);
    const QFont ui = tokens::font(tokens::control);
    const int nameRoom = int(furthest + tokens::space::s8 - nameLeft);
    p.setFont(ui);
    p.setPen(tokens::color(tokens::ink));
    p.drawText(QRectF(nameLeft, 0, qMax(0, nameRoom), height()), Qt::AlignLeft | Qt::AlignVCenter,
               QFontMetrics(ui).elidedText(m_name, Qt::ElideRight, qMax(0, nameRoom)));
}

// ================================================================ QcView

QcView::QcView(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("qcView"));
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);

    // Over the rows while a check runs or after it failed.
    m_statusLine = new QWidget;
    m_statusLine->setObjectName(QStringLiteral("qcStatusLine"));
    m_statusLine->setFixedHeight(kRowHeight);
    auto *line = new QHBoxLayout(m_statusLine);
    line->setContentsMargins(kPad, 0, kPad, 0);
    line->setSpacing(tokens::space::s8);
    m_lineText = shell::text(QString(), tokens::meta, tokens::ink3);
    m_lineText->setObjectName(QStringLiteral("qcStatusLineText"));
    line->addWidget(m_lineText);
    m_lineProgress = new ui::ThinProgress;
    m_lineProgress->setFixedHeight(4);
    line->addWidget(m_lineProgress, 1);
    layout->addWidget(m_statusLine);

    m_scroll = new ui::FadeScrollArea;
    m_scroll->setObjectName(QStringLiteral("qcRows"));
    m_scroll->setFadeColor(tokens::color(tokens::card));
    QPalette card = m_scroll->palette();
    card.setColor(QPalette::Window, tokens::color(tokens::card));
    card.setColor(QPalette::Base, tokens::color(tokens::card));
    m_scroll->viewport()->setPalette(card);
    m_scroll->viewport()->setAutoFillBackground(true);
    m_rows = new QWidget;
    m_rows->setPalette(card);
    m_rows->setAutoFillBackground(true);
    m_rowLayout = new QVBoxLayout(m_rows);
    m_rowLayout->setContentsMargins(0, 0, 0, 0);
    m_rowLayout->setSpacing(0);
    m_rowLayout->setAlignment(Qt::AlignTop);
    m_scroll->setWidget(m_rows);
    layout->addWidget(m_scroll, 1);

    // Before a result: what is happening, and Run QC.
    m_empty = new QWidget;
    m_empty->setObjectName(QStringLiteral("qcEmpty"));
    auto *empty = new QVBoxLayout(m_empty);
    empty->setContentsMargins(tokens::space::s16, 0, tokens::space::s16, tokens::space::s16);
    empty->setSpacing(tokens::space::s8);
    empty->addStretch(1);
    m_status = shell::text(QString(), tokens::meta, tokens::ink3);
    m_status->setObjectName(QStringLiteral("qcStatus"));
    m_status->setAlignment(Qt::AlignCenter);
    m_status->setWordWrap(true);
    empty->addWidget(m_status);
    m_progress = new ui::ThinProgress;
    m_progress->setFixedSize(120, 4);
    empty->addWidget(m_progress, 0, Qt::AlignHCenter);
    m_run = new ui::Button(QStringLiteral("Run QC"), ui::Button::Kind::Secondary);
    m_run->setObjectName(QStringLiteral("qcRun"));
    m_run->setTip(QStringLiteral("Run QC on the whole sequence"));
    connect(m_run, &QAbstractButton::clicked, this, [this] {
        if (m_runAction) m_runAction->trigger();
        else if (m_bus) m_bus->run(QStringLiteral("qc.run"));
    });
    empty->addWidget(m_run, 0, Qt::AlignHCenter);
    empty->addStretch(1);
    layout->addWidget(m_empty, 1);

    if (m_bus) {
        connect(m_bus, &EditorBus::event, this, [this](const QString &type, const QJsonObject &event) {
            if (type == QStringLiteral("check")) setCheck(event);
            else if (type == QStringLiteral("connection")) connectionChanged();
        });
        connect(this, &QcView::threadRequested, m_bus, &EditorBus::openThread);
        const QJsonObject last = m_bus->lastEvent(QStringLiteral("check"));
        if (!last.isEmpty()) m_check = last;
        if (last.value(QStringLiteral("status")).toString() == QStringLiteral("done")) m_result = last;
    }
    rebuild();
}

void QcView::setRunAction(QAction *action) {
    if (m_runAction) disconnect(m_runAction, nullptr, this, nullptr);
    m_runAction = action;
    if (action) connect(action, &QAction::changed, this, &QcView::updateStatus);
    updateStatus();
}

void QcView::setCheck(const QJsonObject &check) {
    m_check = check;
    const QString status = check.value(QStringLiteral("status")).toString();
    // A finished check (status done, or an older bridge's event without one).
    const bool finished = status == QStringLiteral("done") || (status.isEmpty() && check.contains(QStringLiteral("summary")));
    if (finished) {
        m_result = check;
        rebuild();
    } else {
        updateStatus();
    }
}

void QcView::connectionChanged() {
    // A check runs on one connection: when it goes (or a new one comes) the
    // run's last event may never arrive, so stop waiting for it and show
    // the last result.
    if (!running()) return;
    m_check = m_result;
    updateStatus();
}

bool QcView::running() const {
    return m_check.value(QStringLiteral("status")).toString() == QStringLiteral("running");
}

double QcView::progress() const {
    return running() ? std::clamp(m_check.value(QStringLiteral("progress")).toDouble(), 0.0, 1.0) : 0.0;
}

QString QcView::summaryText() const {
    const QJsonObject summary = m_result.value(QStringLiteral("summary")).toObject();
    if (summary.isEmpty()) return {};
    return QStringLiteral("%1 of %2 pass").arg(summary.value(QStringLiteral("passed")).toInt()).arg(summary.value(QStringLiteral("total")).toInt());
}

QString QcView::standardsText() const {
    const QJsonArray rows = m_result.value(QStringLiteral("summary")).toObject().value(QStringLiteral("rows")).toArray();
    bool loudness = false, gamut = false, copy = false, captions = false;
    for (const QJsonValue &value : rows) {
        const QString code = value.toObject().value(QStringLiteral("code")).toString();
        loudness = loudness || code.startsWith(QStringLiteral("loudness."));
        gamut = gamut || code == QStringLiteral("picture.out_of_range");
        // An HDR sequence's: what it read is not what is mastered.
        copy = copy || (code == QStringLiteral("picture.out_of_range") && value.toObject().value(QStringLiteral("name")).toString().contains(QStringLiteral("SDR copy")));
        captions = captions || code.startsWith(QStringLiteral("caption.")) || code == QStringLiteral("text.overlap");
    }
    QStringList parts;
    if (loudness) parts << QStringLiteral("R128");
    if (gamut) parts << (copy ? QStringLiteral("SDR copy") : QStringLiteral("Rec.709"));
    if (captions) parts << QStringLiteral("captions");
    return parts.join(QStringLiteral(" · "));
}

int QcView::rowCount() const {
    return int(m_rowWidgets.size());
}

QString QcView::rowName(int row) const {
    return row >= 0 && row < m_rowWidgets.size() ? m_rowWidgets.at(row)->name() : QString();
}

QString QcView::rowValue(int row) const {
    return row >= 0 && row < m_rowWidgets.size() ? m_rowWidgets.at(row)->value() : QString();
}

bool QcView::rowPassed(int row) const {
    return row >= 0 && row < m_rowWidgets.size() && m_rowWidgets.at(row)->passed();
}

QString QcView::rowThread(int row) const {
    return row >= 0 && row < m_rowWidgets.size() ? m_rowWidgets.at(row)->thread() : QString();
}

QColor QcView::rowValueColor(int row) const {
    return row >= 0 && row < m_rowWidgets.size() ? m_rowWidgets.at(row)->valueColor() : QColor();
}

QString QcView::statusText() const {
    if (!m_empty->isHidden() && !m_status->isHidden()) return m_status->text();
    if (!m_statusLine->isHidden()) return m_lineText->text();
    return {};
}

void QcView::rebuild() {
    for (QcRow *row : std::as_const(m_rowWidgets)) delete row;
    m_rowWidgets.clear();
    const QJsonArray rows = m_result.value(QStringLiteral("summary")).toObject().value(QStringLiteral("rows")).toArray();
    for (const QJsonValue &value : rows) {
        auto *row = new QcRow(value.toObject(), m_rows);
        connect(row, &QcRow::threadRequested, this, &QcView::threadRequested);
        m_rowLayout->addWidget(row);
        m_rowWidgets << row;
    }
    updateStatus();
    m_scroll->updateFades();
}

void QcView::updateStatus() {
    const QString status = m_check.value(QStringLiteral("status")).toString();
    const bool hasRows = !m_rowWidgets.isEmpty();
    const bool isRunning = running();
    const bool failed = status == QStringLiteral("failed");
    QString text;
    if (isRunning) text = QStringLiteral("Running QC… %1%").arg(int(std::lround(progress() * 100)));
    else if (failed) {
        text = m_check.value(QStringLiteral("message")).toString();
        if (text.isEmpty()) text = QStringLiteral("QC did not finish");
    } else if (!hasRows) {
        text = m_result.isEmpty() ? QStringLiteral("QC has not run") : QStringLiteral("QC found nothing to check");
    }
    m_scroll->setVisible(hasRows);
    m_empty->setVisible(!hasRows);
    m_statusLine->setVisible(hasRows && !text.isEmpty());
    m_lineText->setText(text);
    m_lineProgress->setVisible(isRunning);
    m_lineProgress->setValue(progress());
    m_status->setText(text);
    m_status->setVisible(!text.isEmpty());
    m_progress->setVisible(isRunning);
    m_progress->setValue(progress());
    m_run->setVisible(!isRunning);
    // Run QC follows the registry's qc.run; without it, the bus.
    if (m_runAction) {
        m_run->setEnabled(m_runAction->isEnabled() && !isRunning);
        m_run->setToolTip(m_runAction->toolTip());
    } else {
        m_run->setEnabled(m_bus != nullptr && !isRunning);
    }
}

} // namespace kerf
