#include "change_reel.h"

#include "components.h"
#include "design_tokens.h"

#include <QHBoxLayout>
#include <QLabel>
#include <QPaintEvent>
#include <QPainter>
#include <QPalette>
#include <QRegularExpression>
#include <QTimer>

#include <algorithm>
#include <cmath>

namespace kerf {

using namespace tokens;

namespace reel {

QString changeTitle(const ReviewModel::ClipChange &change) {
    const QString name = change.name.isEmpty() ? change.clip : change.name;
    if (change.transition) return QStringLiteral("Transition");
    if (change.added) return QStringLiteral("Add %1").arg(name);
    if (change.removed) return QStringLiteral("Remove %1").arg(name);
    if (change.trimmed) return QStringLiteral("Trim %1").arg(name);
    if (change.moved || change.retracked) return QStringLiteral("Move %1").arg(name);
    if (change.effects) return QStringLiteral("Effects on %1").arg(name);
    return QStringLiteral("Change %1").arg(name);
}

QList<Segment> plan(const QList<ReviewModel::ClipChange> &changes, double beforeDuration, double afterDuration, double handle) {
    QList<Segment> raw;
    for (const ReviewModel::ClipChange &change : changes) {
        Segment segment;
        // What is gone has no place on the proposal but where it was, and what
        // is new none on the program but where it lands.
        segment.after = change.removed ? change.oldStart : change.newStart;
        segment.afterEnd = change.removed ? change.oldStart : change.newEnd;
        segment.before = change.added ? change.newStart : change.oldStart;
        segment.beforeEnd = change.added ? change.newStart : change.oldEnd;
        segment.title = changeTitle(change);
        raw << segment;
    }
    std::stable_sort(raw.begin(), raw.end(), [](const Segment &a, const Segment &b) { return a.after < b.after; });
    QList<Segment> out;
    int merged = 0;
    const auto closeTitle = [&] {
        if (!out.isEmpty() && merged > 0) out.last().title += QStringLiteral(" and %1 more").arg(merged);
        merged = 0;
    };
    for (Segment segment : std::as_const(raw)) {
        segment.after = std::max(0.0, segment.after - handle);
        segment.afterEnd = std::min(std::max(afterDuration, 0.0), segment.afterEnd + handle);
        segment.before = std::max(0.0, segment.before - handle);
        segment.beforeEnd = std::min(std::max(beforeDuration, 0.0), segment.beforeEnd + handle);
        if (!out.isEmpty() && segment.after <= out.last().afterEnd) {
            Segment &last = out.last();
            last.afterEnd = std::max(last.afterEnd, segment.afterEnd);
            last.before = std::min(last.before, segment.before);
            last.beforeEnd = std::max(last.beforeEnd, segment.beforeEnd);
            ++merged;
            continue;
        }
        closeTitle();
        out << segment;
    }
    closeTitle();
    // A change at the very end can leave nothing after the clamp.
    out.erase(std::remove_if(out.begin(), out.end(), [](const Segment &segment) { return segment.length() <= 0.0; }), out.end());
    return out;
}

double total(const QList<Segment> &segments) {
    double sum = 0.0;
    for (const Segment &segment : segments) sum += segment.length();
    return sum;
}

double parseTime(const QString &text, double fps, double sequenceStart) {
    const QString time = text.trimmed();
    static const QRegularExpression timecode(QStringLiteral("^(\\d+):(\\d+):(\\d+)[:;](\\d+)$"));
    const QRegularExpressionMatch match = timecode.match(time);
    if (match.hasMatch()) {
        const double seconds = match.captured(1).toInt() * 3600.0 + match.captured(2).toInt() * 60.0 + match.captured(3).toInt() +
                               match.captured(4).toInt() / std::max(1.0, fps);
        return seconds - sequenceStart;
    }
    bool ok = false;
    if (time.endsWith(QLatin1Char('f'))) {
        const double frames = time.chopped(1).toDouble(&ok);
        return ok ? frames / std::max(1.0, fps) : -1.0;
    }
    const double seconds = (time.endsWith(QLatin1Char('s')) ? time.chopped(1) : time).toDouble(&ok);
    return ok ? seconds : -1.0;
}

} // namespace reel

ChangeReel::ChangeReel(Host host, QObject *parent) : QObject(parent), m_host(std::move(host)) {}

bool ChangeReel::start(const QString &proposalId, const QList<reel::Segment> &segments) {
    if (segments.isEmpty()) return false;
    const bool sameProposal = m_previewing && proposalId == m_proposal;
    m_proposal = proposalId;
    m_segments = segments;
    m_index = 0;
    m_side = Side::After;
    m_finished = false;
    m_rendered = sameProposal && m_rendered;
    m_limit = 0.0;
    for (const reel::Segment &segment : segments) m_limit = std::max({m_limit, segment.afterEnd, segment.beforeEnd});
    show();
    emit changed();
    return true;
}

void ChangeReel::stop() {
    if (!isActive()) return;
    ++m_entry;
    m_segments.clear();
    m_waiting = false;
    m_finished = false;
    m_rendered = false;
    m_host.play(false);
    if (m_previewing) m_host.exitPreview();
    m_previewing = false;
    emit changed();
}

void ChangeReel::next() {
    if (!isActive() || m_index + 1 >= count()) return;
    ++m_index;
    m_finished = false;
    show();
    emit changed();
}

void ChangeReel::previous() {
    if (!isActive() || m_index <= 0) return;
    --m_index;
    m_finished = false;
    show();
    emit changed();
}

void ChangeReel::setSide(Side side) {
    if (!isActive() || side == m_side) return;
    emit sideChanging();
    m_side = side;
    m_finished = false;
    show();
    emit changed();
}

void ChangeReel::show() {
    ++m_entry;
    const reel::Segment &segment = m_segments.at(m_index);
    if (m_side == Side::Before) {
        if (m_previewing) {
            // The program comes back once playback has reopened it.
            m_previewing = false;
            m_rendered = false;
            m_waiting = true;
            m_host.play(false);
            m_host.exitPreview();
            return;
        }
        enterSegment();
        return;
    }
    if (m_rendered && segment.after >= m_renderedStart && segment.afterEnd <= m_renderedEnd) {
        enterSegment();
        return;
    }
    // All the segments when they fit in one render, else this one.
    const double first = m_segments.constFirst().after;
    const double last = m_segments.constLast().afterEnd;
    const bool whole = last - first <= kMostRendered;
    m_renderedStart = whole ? first : segment.after;
    m_renderedEnd = whole ? last : std::min(segment.afterEnd, segment.after + kMostRendered);
    m_rendered = false;
    m_waiting = true;
    m_host.play(false);
    if (!m_host.preview(m_proposal, m_renderedStart, m_renderedEnd - m_renderedStart)) {
        m_waiting = false;
        stop();
    }
}

void ChangeReel::enterSegment() {
    if (!isActive()) return;
    m_waiting = false;
    const reel::Segment &segment = m_segments.at(m_index);
    const bool after = m_side == Side::After;
    m_segmentEnd = after ? segment.afterEnd : segment.beforeEnd;
    m_host.seek(after ? segment.after : segment.before, m_limit);
    if (after) {
        m_host.play(true);
    } else {
        // The program has only just reopened: let the seek land first.
        const int entry = m_entry;
        QTimer::singleShot(300, this, [this, entry] {
            if (entry == m_entry && isActive()) m_host.play(true);
        });
    }
    emit changed();
}

void ChangeReel::finish() {
    m_finished = true;
    m_host.play(false);
    emit changed();
}

void ChangeReel::followPreview(const QJsonObject &event) {
    if (!isActive()) return;
    const QString status = event.value(QStringLiteral("status")).toString();
    if (status == QStringLiteral("running")) return;
    if (status == QStringLiteral("ready") || status == QStringLiteral("live")) {
        // The program's own ready, after an exit, names no proposal.
        const QString id = event.value(QStringLiteral("proposalId")).toString();
        if (m_side == Side::After && m_waiting && !id.isEmpty()) {
            m_previewing = true;
            m_rendered = true;
            enterSegment();
        } else if (m_side == Side::Before && m_waiting && id.isEmpty()) {
            enterSegment();
        }
        return;
    }
    if (status == QStringLiteral("exited")) {
        // Exit preview, from outside: watching a preview that is gone ends.
        if (m_side == Side::After && !m_waiting) {
            m_previewing = false;
            stop();
            return;
        }
        m_previewing = false;
        m_rendered = false;
        if (m_side == Side::Before && m_waiting) {
            // Playback says when it has reopened the program, if it had
            // anything else open; this is for when it did not.
            const int entry = m_entry;
            QTimer::singleShot(1500, this, [this, entry] {
                if (entry == m_entry && m_waiting) enterSegment();
            });
        }
        return;
    }
    if (status == QStringLiteral("failed") || status == QStringLiteral("stopped")) stop();
}

void ChangeReel::followPlayhead(double seconds) {
    if (!isActive() || m_waiting || m_finished) return;
    if (seconds + 1e-3 < m_segmentEnd) return;
    if (m_index + 1 >= count()) {
        finish();
        return;
    }
    ++m_index;
    show();
    emit changed();
}

double ChangeReel::remaining(double playhead) const {
    if (!isActive() || m_finished) return 0.0;
    double left = std::max(0.0, m_segmentEnd - playhead);
    if (m_waiting) left = m_segments.at(m_index).length();
    for (int i = m_index + 1; i < count(); ++i) left += m_segments.at(i).length();
    return left;
}

ChangeReelBar::ChangeReelBar(ChangeReel *reel, QWidget *parent) : QWidget(parent), m_reel(reel) {
    setObjectName(QStringLiteral("changeReelBar"));
    auto *row = new QHBoxLayout(this);
    row->setContentsMargins(space::s12, 0, space::s8, 0);
    row->setSpacing(space::s8);
    m_count = new QLabel;
    m_count->setObjectName(QStringLiteral("changeReelCount"));
    m_count->setFont(tokens::font(timecode));
    QPalette countPalette = m_count->palette();
    countPalette.setColor(QPalette::WindowText, color(accentInk));
    m_count->setPalette(countPalette);
    row->addWidget(m_count);
    m_title = new QLabel;
    m_title->setObjectName(QStringLiteral("changeReelTitle"));
    m_title->setFont(tokens::font(control));
    m_title->setMinimumWidth(0);
    m_title->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    QPalette titlePalette = m_title->palette();
    titlePalette.setColor(QPalette::WindowText, color(ink2));
    m_title->setPalette(titlePalette);
    row->addWidget(m_title, 1);
    m_previous = new ui::IconButton(QStringLiteral("g-chevl"));
    m_previous->setObjectName(QStringLiteral("changeReelPrevious"));
    m_previous->setAccessibleName(QStringLiteral("Previous change"));
    m_previous->setTip(QStringLiteral("Previous change"));
    connect(m_previous, &QAbstractButton::clicked, reel, &ChangeReel::previous);
    row->addWidget(m_previous);
    m_next = new ui::IconButton(QStringLiteral("g-chevr"));
    m_next->setObjectName(QStringLiteral("changeReelNext"));
    m_next->setAccessibleName(QStringLiteral("Next change"));
    m_next->setTip(QStringLiteral("Next change"));
    connect(m_next, &QAbstractButton::clicked, reel, &ChangeReel::next);
    row->addWidget(m_next);
    m_sides = new ui::SegmentedControl;
    m_sides->setObjectName(QStringLiteral("changeReelSides"));
    m_sides->addSegment(QStringLiteral("Before"), QStringLiteral("The program as it is"));
    m_sides->addSegment(QStringLiteral("After"), QStringLiteral("What the proposal makes"));
    m_sides->setCurrentIndex(1, false);
    connect(m_sides, &ui::SegmentedControl::currentChanged, reel, [reel](int index) {
        if (index >= 0) reel->setSide(index == 0 ? ChangeReel::Side::Before : ChangeReel::Side::After);
    });
    row->addWidget(m_sides);
    m_exit = new ui::GhostButton(QStringLiteral("Exit"));
    m_exit->setObjectName(QStringLiteral("changeReelExit"));
    connect(m_exit, &QAbstractButton::clicked, reel, &ChangeReel::stop);
    row->addWidget(m_exit);
}

QString ChangeReelBar::countText() const { return m_count->text(); }

QString ChangeReelBar::titleText() const { return m_title->text(); }

void ChangeReelBar::update(double playhead) {
    if (!m_reel->isActive()) return;
    m_count->setText(QStringLiteral("%1 / %2").arg(m_reel->index() + 1).arg(m_reel->count()));
    const QString title = m_reel->segments().at(m_reel->index()).title;
    if (m_reel->isFinished()) m_title->setText(QStringLiteral("Watched every change"));
    else if (m_reel->isWaiting()) m_title->setText(QStringLiteral("%1 · getting the picture").arg(title));
    else m_title->setText(QStringLiteral("%1 · %2 s left").arg(title).arg(int(std::ceil(m_reel->remaining(playhead)))));
    m_previous->setEnabled(m_reel->index() > 0);
    m_next->setEnabled(m_reel->index() + 1 < m_reel->count());
    const int side = m_reel->side() == ChangeReel::Side::Before ? 0 : 1;
    if (m_sides->currentIndex() != side) {
        const QSignalBlocker blocker(m_sides);
        m_sides->setCurrentIndex(side, false);
    }
}

void ChangeReelBar::paintEvent(QPaintEvent *event) {
    Q_UNUSED(event)
    // A strip floating over the picture.
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    QColor fill = color(chrome);
    fill.setAlphaF(0.94f);
    p.setPen(Qt::NoPen);
    p.setBrush(fill);
    p.drawRoundedRect(rect(), radius::panel, radius::panel);
    ui::paintInsetLine(p, QRectF(rect()), radius::panel, color(line2));
}

} // namespace kerf
