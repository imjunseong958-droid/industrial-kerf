#include "proposal_model.h"

#include <QJsonValue>
#include <QRegularExpression>

#include <algorithm>
#include <cmath>
#include <limits>
#include <utility>

namespace kerf {

namespace {

constexpr double kTimeEpsilon = 1e-6;

QDateTime readTime(const QJsonValue &value) {
    const QString text = value.toString();
    if (text.isEmpty()) return {};
    QDateTime time = QDateTime::fromString(text, Qt::ISODateWithMs);
    if (!time.isValid()) time = QDateTime::fromString(text, Qt::ISODate);
    return time;
}

qint64 readInteger(const QJsonValue &value, qint64 fallback = -1) {
    return value.isDouble() ? qint64(value.toDouble()) : fallback;
}

// The first line worth showing of a proposal script: not blank, not the
// `use <sequence>` that only picks where it runs.
QString scriptSummary(const QString &script) {
    for (QString line : script.split(QLatin1Char('\n'))) {
        line = line.trimmed();
        while (line.startsWith(QLatin1Char('#'))) line = line.mid(1).trimmed();
        if (line.isEmpty() || line.startsWith(QLatin1String("use "))) continue;
        return line;
    }
    return {};
}

struct ClipState {
    QString track;
    QString name;
    double start = 0.0;
    double end = 0.0;
    QJsonValue effects;
    QJsonValue props;
    QJsonValue content; // text, fx, source and its in/out: what the clip shows
    bool enabled = true;
    QJsonValue timing;  // speed and reverse
    QString link;
    int order = 0;
};

struct TransitionState {
    QString track;
    double start = 0.0;
    double end = 0.0;
    QJsonValue look; // from, to, fx, version, align
};

QHash<QString, TransitionState> transitionsOf(const QJsonObject &timeline, QStringList *order) {
    QHash<QString, TransitionState> transitions;
    for (const QJsonValue &trackValue : timeline.value(QStringLiteral("tracks")).toArray()) {
        const QJsonObject track = trackValue.toObject();
        for (const QJsonValue &value : track.value(QStringLiteral("transitions")).toArray()) {
            const QJsonObject transition = value.toObject();
            const QString id = transition.value(QStringLiteral("id")).toString();
            if (id.isEmpty() || transitions.contains(id)) continue;
            TransitionState state;
            state.track = track.value(QStringLiteral("id")).toString();
            state.start = transition.value(QStringLiteral("start")).toDouble();
            state.end = transition.value(QStringLiteral("end")).toDouble();
            state.look = QJsonArray{transition.value(QStringLiteral("from")), transition.value(QStringLiteral("to")),
                                    transition.value(QStringLiteral("fx")), transition.value(QStringLiteral("version")),
                                    transition.value(QStringLiteral("align"))};
            transitions.insert(id, state);
            if (order) order->append(id);
        }
    }
    return transitions;
}

QHash<QString, ClipState> clipsOf(const QJsonObject &timeline, QStringList *order) {
    QHash<QString, ClipState> clips;
    for (const QJsonValue &trackValue : timeline.value(QStringLiteral("tracks")).toArray()) {
        const QJsonObject track = trackValue.toObject();
        const QString trackId = track.value(QStringLiteral("id")).toString();
        for (const QJsonValue &clipValue : track.value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = clipValue.toObject();
            const QString id = clip.value(QStringLiteral("id")).toString();
            if (id.isEmpty() || clips.contains(id)) continue;
            ClipState state;
            state.track = trackId;
            state.name = clip.value(QStringLiteral("name")).toString();
            state.start = clip.value(QStringLiteral("timelineStart")).toDouble();
            state.end = state.start + clip.value(QStringLiteral("duration")).toDouble();
            state.effects = clip.value(QStringLiteral("effects"));
            state.props = clip.value(QStringLiteral("props"));
            state.content = QJsonArray{clip.value(QStringLiteral("text")), clip.value(QStringLiteral("fx")),
                                       clip.value(QStringLiteral("sourceId")), clip.value(QStringLiteral("in")),
                                       clip.value(QStringLiteral("out"))};
            state.enabled = clip.value(QStringLiteral("enabled")).toBool(true);
            state.timing = QJsonArray{clip.value(QStringLiteral("speed")), clip.value(QStringLiteral("speedRatio")),
                                      clip.value(QStringLiteral("reverse")).toBool(false)};
            state.link = clip.value(QStringLiteral("link")).toString();
            state.order = int(clips.size());
            clips.insert(id, state);
            if (order) order->append(id);
        }
    }
    return clips;
}

} // namespace

double ProposalModel::Proposal::distanceFrom(double at) const {
    if (!hasRange) return std::numeric_limits<double>::infinity();
    if (at < start) return start - at;
    if (at > end) return at - end;
    return 0.0;
}

bool ProposalModel::Proposal::hasVersion(const QString &proposalId) const {
    if (proposalId == id) return true;
    return std::any_of(versions.begin(), versions.end(), [&](const Version &v) { return v.proposalId == proposalId; });
}

ProposalModel::ProposalModel(QObject *parent) : QObject(parent) {}

ProposalModel::Author ProposalModel::readAuthor(const QJsonValue &value) {
    Author author;
    if (value.isString()) {
        author.name = value.toString();
        return author;
    }
    const QJsonObject object = value.toObject();
    author.kind = object.value(QStringLiteral("kind")).toString();
    author.name = object.value(QStringLiteral("name")).toString();
    author.id = object.value(QStringLiteral("id")).toString();
    author.role = object.value(QStringLiteral("role")).toString();
    author.initials = object.value(QStringLiteral("initials")).toString().trimmed();
    return author;
}

QString ProposalModel::initials(const Author &author) {
    if (author.kind == QLatin1String("kerf")) return QStringLiteral("K");
    if (author.kind == QLatin1String("qc")) return QStringLiteral("Q");
    if (!author.initials.isEmpty()) return author.initials.left(2);
    const QStringList words = author.name.split(QRegularExpression(QStringLiteral("[\\s._-]+")), Qt::SkipEmptyParts);
    if (words.isEmpty()) return QStringLiteral("?");
    if (words.size() == 1) {
        const QString word = words.first();
        return word.left(1).toUpper() + word.mid(1, 1).toLower();
    }
    return words.at(0).left(1).toUpper() + words.at(1).left(1).toUpper();
}

QList<ProposalModel::ClipChange> ProposalModel::diffTimelines(const QJsonObject &program, const QJsonObject &proposed) {
    QStringList beforeOrder, afterOrder;
    const QHash<QString, ClipState> before = clipsOf(program, &beforeOrder);
    const QHash<QString, ClipState> after = clipsOf(proposed, &afterOrder);
    QList<ClipChange> changes;
    for (const QString &id : afterOrder) {
        const ClipState &now = after[id];
        ClipChange change;
        change.clip = id;
        change.track = now.track;
        change.name = now.name;
        change.newStart = now.start;
        change.newEnd = now.end;
        const auto was = before.constFind(id);
        if (was == before.constEnd()) {
            change.added = true;
            changes << change;
            continue;
        }
        change.oldStart = was->start;
        change.oldEnd = was->end;
        if (change.name.isEmpty()) change.name = was->name;
        change.moved = std::abs(now.start - was->start) > kTimeEpsilon;
        change.trimmed = std::abs((now.end - now.start) - (was->end - was->start)) > kTimeEpsilon;
        change.effects = now.effects != was->effects;
        change.properties = now.props != was->props || now.content != was->content;
        change.retracked = now.track != was->track;
        change.toggled = now.enabled != was->enabled;
        change.retimed = now.timing != was->timing;
        change.relinked = now.link != was->link;
        if (change.moved || change.trimmed || change.effects || change.properties || change.retracked || change.toggled ||
            change.retimed || change.relinked)
            changes << change;
    }
    for (const QString &id : beforeOrder) {
        if (after.contains(id)) continue;
        const ClipState &was = before[id];
        ClipChange change;
        change.clip = id;
        change.track = was.track;
        change.name = was.name;
        change.removed = true;
        change.oldStart = was.start;
        change.oldEnd = was.end;
        changes << change;
    }

    // Transitions, by id: a crossfade added, removed, moved or restyled.
    QStringList fadesBefore, fadesAfter;
    const QHash<QString, TransitionState> oldFades = transitionsOf(program, &fadesBefore);
    const QHash<QString, TransitionState> newFades = transitionsOf(proposed, &fadesAfter);
    for (const QString &id : fadesAfter) {
        const TransitionState &now = newFades[id];
        ClipChange change;
        change.clip = id;
        change.transition = true;
        change.track = now.track;
        change.newStart = now.start;
        change.newEnd = now.end;
        const auto was = oldFades.constFind(id);
        if (was == oldFades.constEnd()) {
            change.added = true;
            changes << change;
            continue;
        }
        change.oldStart = was->start;
        change.oldEnd = was->end;
        change.moved = std::abs(now.start - was->start) > kTimeEpsilon;
        change.trimmed = std::abs((now.end - now.start) - (was->end - was->start)) > kTimeEpsilon;
        change.properties = now.look != was->look;
        change.retracked = now.track != was->track;
        if (change.moved || change.trimmed || change.properties || change.retracked) changes << change;
    }
    for (const QString &id : fadesBefore) {
        if (newFades.contains(id)) continue;
        const TransitionState &was = oldFades[id];
        ClipChange change;
        change.clip = id;
        change.transition = true;
        change.track = was.track;
        change.removed = true;
        change.oldStart = was.start;
        change.oldEnd = was.end;
        changes << change;
    }
    return changes;
}

// ----------------------------------------------------------------- inputs

void ProposalModel::setProposals(const QJsonArray &proposals, const QJsonObject &proposalTimelines) {
    if (proposals == m_proposalJson && proposalTimelines == m_proposalTimelines) return;
    m_proposalJson = proposals;
    m_proposalTimelines = proposalTimelines;
    rebuild();
}

void ProposalModel::setTimeline(const QJsonObject &timeline) {
    if (timeline == m_timeline) return;
    m_timeline = timeline;
    rebuild();
}

void ProposalModel::setSnapshot(const QJsonObject &snapshot) {
    const QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
    const QJsonArray proposals = snapshot.value(QStringLiteral("proposals")).toArray();
    const QJsonObject timelines = snapshot.value(QStringLiteral("proposalTimelines")).toObject();
    if (timeline == m_timeline && proposals == m_proposalJson && timelines == m_proposalTimelines) return;
    m_timeline = timeline;
    m_proposalJson = proposals;
    m_proposalTimelines = timelines;
    rebuild();
}

void ProposalModel::setPlayhead(double seconds) {
    if (!std::isfinite(seconds) || seconds == m_playhead) return;
    m_playhead = seconds;
    const QStringList order = pinOrder();
    if (order == m_pinOrder) return;
    m_pinOrder = order;
    emit changed();
}

bool ProposalModel::applyEvent(const QString &type, const QJsonObject &event) {
    if (type != QLatin1String("state")) return false;
    const QJsonValue snapshot = event.value(QStringLiteral("snapshot"));
    if (!snapshot.isObject()) return false;
    setSnapshot(snapshot.toObject());
    return true;
}

void ProposalModel::rebuild() {
    // Proposals, each with every id it has gone by.
    m_proposals.clear();
    m_proposalIndex.clear();
    for (const QJsonValue &value : m_proposalJson) {
        const QJsonObject entry = value.toObject();
        Proposal proposal;
        proposal.json = entry;
        proposal.id = entry.value(QStringLiteral("proposalId")).toString();
        if (proposal.id.isEmpty()) continue;
        proposal.status = entry.value(QStringLiteral("status")).toString();
        proposal.stale = entry.value(QStringLiteral("freshness")).toString() == QLatin1String("stale");
        proposal.baseRevision = readInteger(entry.value(QStringLiteral("baseRevision")));
        proposal.appliedRevision = readInteger(entry.value(QStringLiteral("appliedRevision")));
        proposal.script = entry.value(QStringLiteral("script")).toString();
        proposal.source = entry.value(QStringLiteral("source")).toString();
        proposal.documentDiff = entry.value(QStringLiteral("documentDiff")).toString();
        proposal.author = readAuthor(entry.value(QStringLiteral("author")));
        if (proposal.author.kind.isEmpty() && proposal.author.name.isEmpty()) {
            // A proposal from before authors were kept is Kerf's.
            proposal.author.kind = QStringLiteral("kerf");
            proposal.author.name = QStringLiteral("Kerf");
        }
        proposal.version = int(readInteger(entry.value(QStringLiteral("version")), 1));
        proposal.supersedes = entry.value(QStringLiteral("supersedes")).toString();
        for (const QJsonValue &versionValue : entry.value(QStringLiteral("versions")).toArray()) {
            const QJsonObject object = versionValue.toObject();
            Version version;
            version.version = int(readInteger(object.value(QStringLiteral("version")), 1));
            version.proposalId = object.value(QStringLiteral("proposalId")).toString();
            version.script = object.value(QStringLiteral("script")).toString();
            version.documentDiff = object.value(QStringLiteral("documentDiff")).toString();
            if (!version.proposalId.isEmpty()) proposal.versions << version;
        }
        if (proposal.versions.isEmpty())
            proposal.versions << Version{proposal.version, proposal.id, proposal.script, proposal.documentDiff};
        proposal.staged = readTime(entry.value(QStringLiteral("staged")));

        // Its range: the clips its timeline changes against the program's.
        QJsonObject timeline = m_proposalTimelines.value(proposal.id).toObject();
        for (int i = int(proposal.versions.size()) - 1; timeline.isEmpty() && i >= 0; --i)
            timeline = m_proposalTimelines.value(proposal.versions.at(i).proposalId).toObject();
        if (!timeline.isEmpty() && !m_timeline.isEmpty()) proposal.changes = diffTimelines(m_timeline, timeline);
        for (const ClipChange &change : std::as_const(proposal.changes)) {
            const double start = change.added ? change.newStart : change.removed ? change.oldStart : std::min(change.oldStart, change.newStart);
            const double end = change.added ? change.newEnd : change.removed ? change.oldEnd : std::max(change.oldEnd, change.newEnd);
            proposal.start = proposal.hasRange ? std::min(proposal.start, start) : start;
            proposal.end = proposal.hasRange ? std::max(proposal.end, end) : end;
            proposal.hasRange = true;
        }

        const int index = int(m_proposals.size());
        if (m_proposalIndex.contains(proposal.id)) continue;
        m_proposalIndex.insert(proposal.id, index);
        for (const Version &version : std::as_const(proposal.versions))
            if (!m_proposalIndex.contains(version.proposalId)) m_proposalIndex.insert(version.proposalId, index);
        if (!proposal.supersedes.isEmpty() && !m_proposalIndex.contains(proposal.supersedes))
            m_proposalIndex.insert(proposal.supersedes, index);
        m_proposals << proposal;
    }

    for (Proposal &proposal : m_proposals) {
        proposal.summary = scriptSummary(proposal.script);
        if (proposal.summary.isEmpty()) proposal.summary = QStringLiteral("Proposal v%1").arg(proposal.version);
    }
    m_pinOrder = pinOrder();
    emit changed();
}

QStringList ProposalModel::pinOrder() const {
    QList<int> open;
    for (int i = 0; i < m_proposals.size(); ++i)
        if (m_proposals.at(i).isOpen()) open << i;
    std::stable_sort(open.begin(), open.end(), [this](int a, int b) {
        return m_proposals.at(a).distanceFrom(m_playhead) < m_proposals.at(b).distanceFrom(m_playhead);
    });
    QStringList ids;
    for (const int index : std::as_const(open)) ids << m_proposals.at(index).id;
    return ids;
}

const ProposalModel::Proposal *ProposalModel::proposal(const QString &id) const {
    const auto found = m_proposalIndex.constFind(id);
    return found == m_proposalIndex.constEnd() ? nullptr : &m_proposals.at(*found);
}

QList<ProposalModel::Proposal> ProposalModel::openProposals() const {
    QList<Proposal> open;
    for (const QString &id : m_pinOrder)
        if (const Proposal *entry = proposal(id)) open << *entry;
    return open;
}

QString ProposalModel::pinnedProposal() const { return m_pinOrder.isEmpty() ? QString() : m_pinOrder.first(); }

bool ProposalModel::isPinned(const QString &proposalId) const {
    const Proposal *entry = proposal(proposalId);
    return entry && !m_pinOrder.isEmpty() && entry->id == m_pinOrder.first();
}

} // namespace kerf
