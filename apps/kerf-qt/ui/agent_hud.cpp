#include "agent_hud.h"
#include "components.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "editor_common.h"
#include "icons.h"
#include "review_model.h"

#include <QAbstractSpinBox>
#include <QApplication>
#include <QFontMetricsF>
#include <QJsonArray>
#include <QJsonObject>
#include <QKeyEvent>
#include <QLineEdit>
#include <QPainter>
#include <QPainterPath>
#include <QPlainTextEdit>
#include <QTextEdit>

#include <algorithm>
#include <cmath>

namespace kerf {

using tokens::color;

namespace {

// The card's fill: rgba(28,28,28,.94).
QColor hudFill() { return QColor(28, 28, 28, tokens::alpha(0.94)); }

bool editsText(const QObject *object) {
    return qobject_cast<const QLineEdit *>(object) || qobject_cast<const QTextEdit *>(object) ||
           qobject_cast<const QPlainTextEdit *>(object) || qobject_cast<const QAbstractSpinBox *>(object);
}

} // namespace

AgentHud::AgentHud(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("agentHud"));
    setAccessibleName(QStringLiteral("Preview render"));
    m_bar = new ui::ThinProgress(this);
    m_bar->setObjectName(QStringLiteral("agentHudProgress"));
    m_stop = new ui::Button(QStringLiteral("Stop"), ui::Button::Kind::Secondary, this);
    m_stop->setObjectName(QStringLiteral("agentHudStop"));
    m_stop->setTip(QStringLiteral("Stop preview render"), QStringLiteral("Esc"));
    connect(m_stop, &QAbstractButton::clicked, this, &AgentHud::stop);
    resize(sizeHint());
    hide();
    if (m_bus) {
        connect(m_bus, &EditorBus::event, this, &AgentHud::handle);
        connect(m_bus, &EditorBus::snapshotChanged, this, &AgentHud::refresh);
        connect(m_bus, &EditorBus::reviewChanged, this, &AgentHud::refresh);
    }
    relayout();
}

AgentHud::~AgentHud() {
    if (m_filtering && qApp) qApp->removeEventFilter(this);
}

QAbstractButton *AgentHud::stopButton() const { return m_stop; }

QSize AgentHud::sizeHint() const {
    return QSize(kCardWidth, kCardHeight);
}

QRectF AgentHud::cardRect(const QSizeF &picture) {
    const double w = picture.width(), h = picture.height();
    const double tx = std::round(w * 0.05), ty = std::round(h * 0.05);
    const double hw = std::max(0.0, std::min<double>(kCardWidth, w - 2 * tx - 16));
    return QRectF(w - tx - 8 - hw, ty + 8, hw, kCardHeight);
}

void AgentHud::handle(const QString &type, const QJsonObject &event) {
    if (type == QStringLiteral("preview")) {
        const QString status = event.value(QStringLiteral("status")).toString();
        if (status == QStringLiteral("running")) {
            const QString proposal = event.value(QStringLiteral("proposalId")).toString();
            // A render stopped here stays hidden until its run ends (any
            // other status), unless another proposal's render takes over.
            if (m_running && proposal != m_proposal) m_stopped = false;
            m_running = true;
            m_proposal = proposal;
            m_progress = std::clamp(event.value(QStringLiteral("progress")).toDouble(), 0.0, 1.0);
            m_message = event.value(QStringLiteral("message")).toString().simplified();
        } else if (!status.isEmpty()) {
            m_running = false;
            m_stopped = false;
            m_progress = 0.0;
        }
        refresh();
    } else if (type == QStringLiteral("state")) {
        refresh();
    }
}

void AgentHud::stop() {
    if (!active()) return;
    if (m_bus) m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("preview_stop")}});
    m_stopped = true;
    refresh();
}

int AgentHud::versionOf(const QString &proposalId) const {
    if (proposalId.isEmpty() || !m_bus) return 0;
    if (const ReviewModel *review = m_bus->review())
        if (const ReviewModel::Proposal *proposal = review->proposal(proposalId)) {
            for (const ReviewModel::Version &version : proposal->versions)
                if (version.proposalId == proposalId) return version.version;
            return proposal->version;
        }
    // The snapshot's own entry: the proposal, or one of its versions.
    for (const QJsonValue &value : m_bus->snapshot().value(QStringLiteral("proposals")).toArray()) {
        const QJsonObject entry = value.toObject();
        for (const QJsonValue &v : entry.value(QStringLiteral("versions")).toArray())
            if (v.toObject().value(QStringLiteral("proposalId")).toString() == proposalId)
                return v.toObject().value(QStringLiteral("version")).toInt();
        if (entry.value(QStringLiteral("proposalId")).toString() == proposalId)
            return entry.value(QStringLiteral("version")).toInt(1);
    }
    return 0;
}

QString AgentHud::title() const {
    const int version = versionOf(m_proposal);
    return version > 0 ? QStringLiteral("Rendering preview · v%1").arg(version) : QStringLiteral("Rendering preview");
}

QString AgentHud::text() const { return active() ? title() : QString(); }

QList<AgentHud::Run> AgentHud::detailRuns() const {
    const ReviewModel *review = m_bus ? m_bus->review() : nullptr;
    const ReviewModel::Proposal *proposal = review && !m_proposal.isEmpty() ? review->proposal(m_proposal) : nullptr;
    if (proposal && !proposal->changes.isEmpty()) {
        const ReviewModel::ClipChange &first = proposal->changes.first();
        const QString name = first.name.isEmpty() ? first.clip : first.name;
        const qsizetype n = proposal->changes.size();
        const QString count = n == 1 ? QStringLiteral(" · 1 change") : QStringLiteral(" · %1 changes").arg(n);
        if (first.removed) return {{name + QStringLiteral(" removed") + count, false}};
        const double fps = m_bus ? m_bus->fps() : 24.0;
        return {{name + QStringLiteral(" → "), false}, {timelineTimecode(first.newStart, fps), true}, {count, false}};
    }
    if (proposal && !proposal->summary.isEmpty()) return {{proposal->summary.section(QLatin1Char('\n'), 0, 0), false}};
    if (!m_message.isEmpty()) return {{m_message, false}};
    return {};
}

QString AgentHud::detail() const {
    QString out;
    for (const Run &run : detailRuns()) out += run.text;
    return out;
}

QString AgentHud::percentText() const { return QStringLiteral("%1%").arg(int(std::lround(m_progress * 100.0))); }

void AgentHud::setQuiet(bool quiet) {
    if (quiet == m_quiet) return;
    m_quiet = quiet;
    refresh();
}

void AgentHud::refresh() {
    m_bar->setValue(m_progress);
    const bool now = active();
    setVisible(now && !m_quiet);
    if (now) raise();
    relayout();
    update();
    updateEscape();
    if (now != m_wasActive) {
        m_wasActive = now;
        emit activeChanged(now);
    }
}

void AgentHud::relayout() {
    const int pctWidth = ui::snapWidth(tokens::monoSmall, percentText());
    const QSize stop = m_stop->sizeHint();
    // pad 8 12; row 3 (24 tall) at y 40, 24 in from the content's left.
    const int left = 12 + 24, right = width() - 12;
    const int stopX = right - stop.width();
    const int pctX = stopX - 8 - pctWidth;
    m_stop->move(stopX, 40);
    m_bar->setGeometry(left, 40 + 10, std::max(0, pctX - 8 - left), 4);
}

void AgentHud::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    relayout();
}

void AgentHud::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF r = rect();
    QPainterPath card;
    card.addRoundedRect(r, tokens::radius::card, tokens::radius::card);
    p.fillPath(card, hudFill());
    ui::paintInsetLine(p, r, tokens::radius::card, color(tokens::line2));

    // Row 1: the 16 x 16 Kerf tile (radius 4, on, g-kerf 14 ink) and the title.
    const qreal left = 12;
    const qreal row1 = 8;
    const QRectF tile(left, row1, 16, 16);
    p.setPen(Qt::NoPen);
    p.setBrush(color(tokens::on));
    p.drawRoundedRect(tile, tokens::radius::small, tokens::radius::small);
    icons::paint(p, QStringLiteral("g-kerf"), tile, color(tokens::ink), 14);
    const qreal textLeft = tile.right() + 8;
    const qreal titleRight = width() - 12;
    const QFont titleFont = tokens::font(tokens::controlStrong);
    p.setFont(titleFont);
    p.setPen(color(tokens::ink));
    p.drawText(QRectF(textLeft, row1, std::max<qreal>(0, titleRight - textLeft), 16), Qt::AlignLeft | Qt::AlignVCenter,
               QFontMetricsF(titleFont).elidedText(title(), Qt::ElideRight, std::max<qreal>(0, titleRight - textLeft)));

    // Row 2 (the card only): 11/16 ink3, the timecode in mono, with an ellipsis.
    {
        qreal x = 12 + 24;
        const qreal limit = width() - 12;
        p.setPen(color(tokens::ink3));
        for (const Run &run : detailRuns()) {
            if (x >= limit) break;
            const QFont font = tokens::font(run.mono ? tokens::monoSmall : tokens::meta);
            const QFontMetricsF metrics(font);
            const QString shown = metrics.elidedText(run.text, Qt::ElideRight, limit - x);
            p.setFont(font);
            p.drawText(QRectF(x, 24, limit - x, 16), Qt::AlignLeft | Qt::AlignVCenter, shown);
            x += metrics.horizontalAdvance(shown);
            if (shown != run.text) break;
        }
    }

    // The percent, mono 11 ink2, between the bar and Stop.
    const QRect bar = m_bar->geometry();
    p.setFont(tokens::font(tokens::monoSmall));
    p.setPen(color(tokens::ink2));
    p.drawText(QRectF(bar.right() + 1 + 8, bar.center().y() - 8, ui::snapWidth(tokens::monoSmall, percentText()), 16),
               Qt::AlignLeft | Qt::AlignVCenter, percentText());
}

void AgentHud::updateEscape() {
    // While it shows, Esc stops the render, ahead of the window's own
    // shortcuts on Esc (it is taken before they are matched).
    const bool want = active() && qApp;
    if (want == m_filtering) return;
    m_filtering = want;
    if (want) qApp->installEventFilter(this);
    else qApp->removeEventFilter(this);
}

bool AgentHud::eventFilter(QObject *watched, QEvent *event) {
    if (event->type() != QEvent::ShortcutOverride && event->type() != QEvent::KeyPress)
        return QWidget::eventFilter(watched, event);
    auto *key = static_cast<QKeyEvent *>(event);
    if (key->key() != Qt::Key_Escape || key->modifiers() != Qt::NoModifier || !active() || !isVisible())
        return QWidget::eventFilter(watched, event);
    // Only for this window, outside text fields, menus and dialogs.
    auto *target = qobject_cast<QWidget *>(watched);
    if (!target || target->window() != window() || editsText(target) || QApplication::activePopupWidget() ||
        QApplication::activeModalWidget())
        return QWidget::eventFilter(watched, event);
    if (event->type() == QEvent::ShortcutOverride) {
        event->accept();
        return true;
    }
    stop();
    return true;
}

} // namespace kerf
