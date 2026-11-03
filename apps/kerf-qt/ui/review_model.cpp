#include "review_model.h"

#include <QJsonValue>
#include <QMap>
#include <QRegularExpression>
#include <QSet>

#include <algorithm>
#include <cmath>
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

double readSeconds(const QJsonValue &value) {
    if (!value.isDouble()) return std::numeric_limits<double>::quiet_NaN();
    const double seconds = value.toDouble();
    return std::isfinite(seconds) ? seconds : std::numeric_limits<double>::quiet_NaN();
}

qint64 readInteger(const QJsonValue &value, qint64 fallback = -1) {
    return value.isDouble() ? qint64(value.toDouble()) : fallback;
}

QStringList readStrings(const QJsonValue &value) {
    QStringList out;
    for (const QJsonValue &item : value.toArray())
        if (item.isString()) out << item.toString();
    return out;
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

QString firstLine(const QString &text) {
    const QString trimmed = text.trimmed();
    const int end = int(trimmed.indexOf(QLatin1Char('\n')));
    return (end < 0 ? trimmed : trimmed.left(end)).trimmed();
}

// -1, 0 or 1: older first. QDateTime keeps milliseconds and kerfd writes
// microseconds, so a tie falls to kerfd's own text, which its fixed RFC 3339
// format sorts as it sorts the log: by (created, id). No time sorts last.
int compareCreated(const ReviewModel::Comment &a, const ReviewModel::Comment &b) {
    if (a.created.isValid() != b.created.isValid()) return a.created.isValid() ? -1 : 1;
    if (a.created != b.created) return a.created < b.created ? -1 : 1;
    if (a.createdText != b.createdText) return a.createdText < b.createdText ? -1 : 1;
    return 0;
}

bool createdBefore(const ReviewModel::Comment &a, const ReviewModel::Comment &b) {
    const int order = compareCreated(a, b);
    if (order != 0) return order < 0;
    return a.id < b.id;
}

bool contains(const QString &haystack, const QString &needle) {
    return haystack.contains(needle, Qt::CaseInsensitive);
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

double ReviewModel::Proposal::distanceFrom(double at) const {
    if (!hasRange) return std::numeric_limits<double>::infinity();
    if (at < start) return start - at;
    if (at > end) return at - end;
    return 0.0;
}

bool ReviewModel::Proposal::hasVersion(const QString &proposalId) const {
    if (proposalId == id) return true;
    return std::any_of(versions.begin(), versions.end(), [&](const Version &v) { return v.proposalId == proposalId; });
}

ReviewModel::ReviewModel(QObject *parent) : QObject(parent) {}

// ---------------------------------------------------------------- reading

ReviewModel::Author ReviewModel::readAuthor(const QJsonValue &value) {
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

ReviewModel::Comment ReviewModel::readComment(const QJsonObject &record) {
    Comment comment;
    comment.json = record;
    comment.id = record.value(QStringLiteral("id")).toString();
    comment.parent = record.value(QStringLiteral("parent")).toString();
    comment.author = readAuthor(record.value(QStringLiteral("author")));
    comment.at = readSeconds(record.value(QStringLiteral("at")));
    const double duration = readSeconds(record.value(QStringLiteral("duration")));
    comment.duration = comment.hasAt() && !std::isnan(duration) && duration > 0.0 ? duration : 0.0;
    comment.frame = int(readInteger(record.value(QStringLiteral("frame"))));
    comment.body = record.value(QStringLiteral("body")).toString();
    comment.blocking = record.value(QStringLiteral("blocking")).toBool();
    for (QString tag : readStrings(record.value(QStringLiteral("tags")))) {
        tag = tag.trimmed();
        while (tag.startsWith(QLatin1Char('#'))) tag = tag.mid(1);
        if (tag.isEmpty()) continue;
        // kerfd's CSV writes the flag as this tag; read either as the flag.
        if (tag.compare(QLatin1String(kBlockingTag), Qt::CaseInsensitive) == 0) {
            comment.blocking = true;
            continue;
        }
        if (!comment.tags.contains(tag, Qt::CaseInsensitive)) comment.tags << tag;
    }
    comment.drawing = record.value(QStringLiteral("drawing")).toArray();
    comment.proposal = record.value(QStringLiteral("proposal")).toString();
    comment.steps = readStrings(record.value(QStringLiteral("steps")));
    comment.error = record.value(QStringLiteral("error")).toBool();
    comment.resolved = record.value(QStringLiteral("resolved")).toBool();
    const QJsonValue resolvedBy = record.value(QStringLiteral("resolvedBy"));
    comment.hasResolvedBy = resolvedBy.isObject() || resolvedBy.isString();
    if (comment.hasResolvedBy) comment.resolvedBy = readAuthor(resolvedBy);
    comment.resolvedRevision = readInteger(record.value(QStringLiteral("resolvedRevision")));
    comment.qcKey = record.value(QStringLiteral("qcKey")).toString();
    comment.createdText = record.value(QStringLiteral("created")).toString();
    comment.created = readTime(record.value(QStringLiteral("created")));
    comment.revision = readInteger(record.value(QStringLiteral("revision")));
    comment.edited = readTime(record.value(QStringLiteral("edited")));
    return comment;
}

QString ReviewModel::initials(const Author &author) {
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

QList<ReviewModel::ClipChange> ReviewModel::diffTimelines(const QJsonObject &program, const QJsonObject &proposed) {
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

void ReviewModel::setComments(const QJsonArray &items) {
    if (items == m_commentJson && !m_commentJson.isEmpty()) return;
    m_commentJson = items;
    QList<Comment> comments;
    comments.reserve(items.size());
    for (const QJsonValue &item : items) {
        Comment comment = readComment(item.toObject());
        if (comment.id.isEmpty()) continue;
        comments << comment;
    }
    // kerfd sends them by (created, id); this sorts the same way, its text
    // breaking the ties QDateTime's milliseconds leave.
    std::stable_sort(comments.begin(), comments.end(), createdBefore);
    m_comments = comments;
    rebuild();
}

void ReviewModel::setProposals(const QJsonArray &proposals, const QJsonObject &proposalTimelines) {
    if (proposals == m_proposalJson && proposalTimelines == m_proposalTimelines) return;
    m_proposalJson = proposals;
    m_proposalTimelines = proposalTimelines;
    rebuild();
}

void ReviewModel::setTimeline(const QJsonObject &timeline) {
    if (timeline == m_timeline) return;
    m_timeline = timeline;
    rebuild();
}

void ReviewModel::setSnapshot(const QJsonObject &snapshot) {
    const QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
    const QJsonArray proposals = snapshot.value(QStringLiteral("proposals")).toArray();
    const QJsonObject timelines = snapshot.value(QStringLiteral("proposalTimelines")).toObject();
    if (timeline == m_timeline && proposals == m_proposalJson && timelines == m_proposalTimelines) return;
    m_timeline = timeline;
    m_proposalJson = proposals;
    m_proposalTimelines = timelines;
    rebuild();
}

void ReviewModel::setIdentity(const QJsonObject &user) {
    if (user == m_identity) return;
    m_identity = user;
    rebuild();
}

void ReviewModel::setPlayhead(double seconds) {
    if (!std::isfinite(seconds) || seconds == m_playhead) return;
    m_playhead = seconds;
    const QStringList order = pinOrder();
    if (order == m_pinOrder) return;
    m_pinOrder = order;
    emit changed();
}

void ReviewModel::setViewAs(const QString &personId) {
    const QString id = personId.trimmed();
    if (id == m_viewAs) return;
    m_viewAs = id;
    emit changed();
}

void ReviewModel::setWelcomeShown(bool shown) {
    if (shown == m_welcome) return;
    m_welcome = shown;
    rebuild();
}

bool ReviewModel::applyEvent(const QString &type, const QJsonObject &event) {
    if (type == QLatin1String("comments")) {
        QJsonValue items = event.value(QStringLiteral("items"));
        if (!items.isArray()) {
            const QJsonValue comments = event.value(QStringLiteral("comments"));
            items = comments.isArray() ? comments : comments.toObject().value(QStringLiteral("items"));
        }
        if (!items.isArray()) return false;
        setComments(items.toArray());
        return true;
    }
    if (type == QLatin1String("state")) {
        const QJsonValue snapshot = event.value(QStringLiteral("snapshot"));
        if (!snapshot.isObject()) return false;
        setSnapshot(snapshot.toObject());
        return true;
    }
    if (type == QLatin1String("collab")) {
        const QJsonObject status = event.value(QStringLiteral("status")).toObject();
        if (!status.contains(QStringLiteral("user"))) return false;
        setIdentity(status.value(QStringLiteral("user")).toObject());
        return true;
    }
    return false;
}

// ---------------------------------------------------------------- filters

void ReviewModel::setChip(Chip chip) {
    if (chip == m_chip) return;
    m_chip = chip;
    emit changed();
}

void ReviewModel::setFrom(From from) {
    if (from == m_from) return;
    m_from = from;
    emit changed();
}

void ReviewModel::setHideResolved(bool hide) {
    if (hide == m_hideResolved) return;
    m_hideResolved = hide;
    emit changed();
}

void ReviewModel::setWithDrawings(bool with) {
    if (with == m_withDrawings) return;
    m_withDrawings = with;
    emit changed();
}

void ReviewModel::setTag(const QString &tag) {
    QString clean = tag.trimmed();
    while (clean.startsWith(QLatin1Char('#'))) clean = clean.mid(1);
    if (clean.compare(QLatin1String(kBlockingTag), Qt::CaseInsensitive) == 0) clean = QString::fromLatin1(kBlockingTag);
    if (clean == m_tag) return;
    m_tag = clean;
    emit changed();
}

void ReviewModel::toggleTag(const QString &tag) {
    QString clean = tag.trimmed();
    while (clean.startsWith(QLatin1Char('#'))) clean = clean.mid(1);
    setTag(clean.compare(m_tag, Qt::CaseInsensitive) == 0 ? QString() : clean);
}

void ReviewModel::setSearch(const QString &text) {
    const QString clean = text.trimmed();
    if (clean == m_search) return;
    m_search = clean;
    emit changed();
}

void ReviewModel::setSort(Sort sort) {
    if (sort == m_sort) return;
    m_sort = sort;
    sortThreads(m_threads);
    m_threadIndex.clear();
    for (int i = 0; i < m_threads.size(); ++i) m_threadIndex.insert(m_threads.at(i).id, i);
    emit changed();
}

void ReviewModel::clearFilters() {
    const bool any = m_chip != Chip::All || filtering();
    m_chip = Chip::All;
    m_from = From::Anyone;
    m_hideResolved = false;
    m_withDrawings = false;
    m_tag.clear();
    m_search.clear();
    if (any) emit changed();
}

bool ReviewModel::filtering() const {
    return m_from != From::Anyone || m_hideResolved || m_withDrawings || !m_tag.isEmpty() || !m_search.isEmpty();
}

QString ReviewModel::sortName(Sort sort) {
    switch (sort) {
    case Sort::Timecode: return QStringLiteral("Timecode");
    case Sort::Newest: return QStringLiteral("Newest");
    case Sort::OpenFirst: return QStringLiteral("Open first");
    case Sort::Commenter: return QStringLiteral("Commenter");
    }
    return {};
}

// ---------------------------------------------------------------- building

void ReviewModel::rebuild() {
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

    // Threads: each comment under its root. A reply whose root is gone (a
    // deleted comment is left out) stands as a thread of its own.
    QHash<QString, int> byId;
    for (int i = 0; i < m_comments.size(); ++i) byId.insert(m_comments.at(i).id, i);
    const auto rootOf = [&](int index) {
        QSet<QString> seen;
        int at = index;
        while (true) {
            const Comment &comment = m_comments.at(at);
            seen.insert(comment.id);
            const auto parent = byId.constFind(comment.parent);
            if (comment.parent.isEmpty() || parent == byId.constEnd() || seen.contains(comment.parent)) return at;
            at = *parent;
        }
    };
    m_threads.clear();
    m_rootOf.clear();
    QHash<QString, int> threadOfRoot;
    for (int i = 0; i < m_comments.size(); ++i) {
        const Comment &comment = m_comments.at(i);
        const Comment &root = m_comments.at(rootOf(i));
        auto found = threadOfRoot.constFind(root.id);
        if (found == threadOfRoot.constEnd()) {
            Thread thread;
            thread.id = root.id;
            thread.root = root;
            found = threadOfRoot.insert(root.id, int(m_threads.size()));
            m_threads << thread;
        }
        if (comment.id != root.id) m_threads[*found].replies << comment;
        m_rootOf.insert(comment.id, root.id);
    }

    for (Thread &thread : m_threads) {
        QList<int> linked;
        bool openBlocker = false;
        const auto look = [&](const Comment &comment) {
            if (comment.hasDrawing()) thread.hasDrawing = true;
            if (comment.blocking) {
                thread.flagged = true;
                if (!comment.resolved) openBlocker = true;
            }
            for (const QString &tag : comment.tags)
                if (!thread.tags.contains(tag, Qt::CaseInsensitive)) thread.tags << tag;
            const auto proposal = m_proposalIndex.constFind(comment.proposal);
            if (comment.proposal.isEmpty() || proposal == m_proposalIndex.constEnd()) return;
            Proposal &entry = m_proposals[*proposal];
            if (entry.threadId.isEmpty()) entry.threadId = thread.id;
            if (entry.threadId == thread.id && entry.summary.isEmpty()) entry.summary = firstLine(comment.body);
            if (!linked.contains(*proposal)) linked << *proposal;
        };
        look(thread.root);
        for (const Comment &reply : std::as_const(thread.replies)) look(reply);
        std::sort(linked.begin(), linked.end());
        for (const int index : std::as_const(linked)) {
            const Proposal &entry = m_proposals.at(index);
            thread.proposals << entry.id;
            if (thread.openProposal.isEmpty() && entry.isOpen()) thread.openProposal = entry.id;
            if (entry.status != QLatin1String("accepted") || entry.appliedRevision < 0 ||
                entry.appliedRevision <= thread.acceptedRevision)
                continue;
            // kerfd's accept resolves the comments carrying the proposal at
            // its applied revision; all of them resolved, one by the accept.
            bool all = true, byAccept = false;
            const auto check = [&](const Comment &comment) {
                if (comment.proposal.isEmpty() || !entry.hasVersion(comment.proposal)) return;
                if (!comment.resolved) all = false;
                else if (comment.resolvedRevision == entry.appliedRevision) byAccept = true;
            };
            check(thread.root);
            for (const Comment &reply : std::as_const(thread.replies)) check(reply);
            if (!all || !byAccept) continue;
            thread.acceptedProposal = entry.id;
            thread.acceptedRevision = entry.appliedRevision;
        }
        const bool settledByAccept = !thread.acceptedProposal.isEmpty() && thread.openProposal.isEmpty();
        thread.open = !thread.root.resolved && !settledByAccept;
        thread.blocking = thread.open && openBlocker;
    }

    // A pending proposal no comment links to gets a thread from its author.
    for (Proposal &proposal : m_proposals) {
        if (proposal.summary.isEmpty()) proposal.summary = scriptSummary(proposal.script);
        if (proposal.summary.isEmpty()) proposal.summary = QStringLiteral("Proposal v%1").arg(proposal.version);
        if (!proposal.threadId.isEmpty() || !proposal.isOpen()) continue;
        Thread thread;
        thread.id = QString::fromLatin1(kProposalThreadPrefix) + proposal.id;
        thread.origin = Origin::Proposal;
        thread.root.id = thread.id;
        thread.root.author = proposal.author;
        thread.root.body = proposal.summary;
        thread.root.proposal = proposal.id;
        thread.root.created = proposal.staged;
        thread.root.createdText = proposal.json.value(QStringLiteral("staged")).toString();
        thread.root.revision = proposal.baseRevision;
        thread.open = true;
        thread.proposals << proposal.id;
        thread.openProposal = proposal.id;
        proposal.threadId = thread.id;
        proposal.synthesised = true;
        m_rootOf.insert(thread.id, thread.id);
        m_threads << thread;
    }

    if (m_welcome) {
        Thread welcome;
        welcome.id = QString::fromLatin1(kWelcomeId);
        welcome.origin = Origin::Welcome;
        welcome.root.id = welcome.id;
        welcome.root.author = Author{QStringLiteral("kerf"), QStringLiteral("Kerf"), {}, {}, {}};
        m_rootOf.insert(welcome.id, welcome.id);
        m_threads << welcome;
    }

    sortThreads(m_threads);
    m_threadIndex.clear();
    for (int i = 0; i < m_threads.size(); ++i) m_threadIndex.insert(m_threads.at(i).id, i);
    for (Proposal &proposal : m_proposals) {
        const Thread *owner = thread(proposal.threadId);
        proposal.blocking = owner && owner->blocking;
    }

    Counts counts;
    counts.all = int(m_threads.size());
    for (const Thread &thread : std::as_const(m_threads)) {
        if (thread.open) ++counts.open;
        if (thread.blocking) ++counts.blocking;
    }
    counts.proposals = int(std::count_if(m_proposals.begin(), m_proposals.end(), [](const Proposal &p) { return p.isOpen(); }));
    const bool countsMoved = counts != m_counts;
    m_counts = counts;
    m_pinOrder = pinOrder();

    const bool currentGone = !m_current.isEmpty() && !m_threadIndex.contains(m_current);
    if (currentGone) m_current.clear();

    emit changed();
    if (countsMoved) emit countsChanged();
    if (currentGone) emit currentChanged(QString());
}

double ReviewModel::sortTime(const Thread &thread) const {
    if (thread.root.hasAt()) return thread.root.at;
    if (thread.origin == Origin::Proposal) {
        const Proposal *entry = proposal(thread.openProposal);
        if (entry && entry->hasRange) return entry->start;
    }
    // A note on the whole sequence comes before the timed ones.
    return -std::numeric_limits<double>::infinity();
}

void ReviewModel::sortThreads(QList<Thread> &threads) const {
    const auto byTime = [this](const Thread &a, const Thread &b) {
        const double ta = sortTime(a), tb = sortTime(b);
        if (ta != tb) return ta < tb;
        return createdBefore(a.root, b.root);
    };
    std::stable_sort(threads.begin(), threads.end(), [&](const Thread &a, const Thread &b) {
        const bool wa = a.origin == Origin::Welcome, wb = b.origin == Origin::Welcome;
        if (wa != wb) return wa;
        switch (m_sort) {
        case Sort::Timecode: return byTime(a, b);
        case Sort::Newest: {
            if (a.root.created.isValid() != b.root.created.isValid()) return a.root.created.isValid();
            const int order = compareCreated(a.root, b.root);
            if (order != 0) return order > 0;
            return byTime(a, b);
        }
        case Sort::OpenFirst:
            if (a.open != b.open) return a.open;
            return byTime(a, b);
        case Sort::Commenter: {
            const int order = QString::compare(displayName(a.root.author), displayName(b.root.author), Qt::CaseInsensitive);
            if (order != 0) return order < 0;
            return byTime(a, b);
        }
        }
        return false;
    });
}

QStringList ReviewModel::pinOrder() const {
    QList<int> open;
    for (int i = 0; i < m_proposals.size(); ++i)
        if (m_proposals.at(i).isOpen()) open << i;
    std::stable_sort(open.begin(), open.end(), [this](int a, int b) {
        const Proposal &pa = m_proposals.at(a), &pb = m_proposals.at(b);
        if (pa.blocking != pb.blocking) return pa.blocking;
        return pa.distanceFrom(m_playhead) < pb.distanceFrom(m_playhead);
    });
    QStringList ids;
    for (const int index : std::as_const(open)) ids << m_proposals.at(index).id;
    return ids;
}

// ---------------------------------------------------------------- outputs

bool ReviewModel::passes(const Thread &thread) const {
    switch (m_chip) {
    case Chip::All: break;
    case Chip::Open:
        if (!thread.open) return false;
        break;
    case Chip::Proposals:
        if (thread.openProposal.isEmpty()) return false;
        break;
    }
    const auto anyAuthor = [&](auto test) {
        if (test(thread.root.author)) return true;
        return std::any_of(thread.replies.begin(), thread.replies.end(), [&](const Comment &c) { return test(c.author); });
    };
    switch (m_from) {
    case From::Anyone: break;
    case From::Kerf:
        if (!anyAuthor([](const Author &a) { return a.isKerf(); })) return false;
        break;
    case From::Agents:
        if (!anyAuthor([](const Author &a) { return a.isAgent(); })) return false;
        break;
    case From::People:
        if (!thread.root.author.isPerson()) return false;
        break;
    }
    if (m_hideResolved && thread.resolved()) return false;
    if (m_withDrawings && !thread.hasDrawing) return false;
    if (!m_tag.isEmpty()) {
        if (m_tag == QLatin1String(kBlockingTag)) {
            if (!thread.flagged) return false;
        } else if (!thread.tags.contains(m_tag, Qt::CaseInsensitive)) {
            return false;
        }
    }
    if (!m_search.isEmpty()) {
        const auto matches = [&](const Comment &c) {
            if (contains(c.body, m_search) || contains(c.author.name, m_search) || contains(c.author.role, m_search))
                return true;
            if (isYou(c.author) && contains(QStringLiteral("You"), m_search)) return true;
            return std::any_of(c.tags.begin(), c.tags.end(), [&](const QString &tag) { return contains(tag, m_search); });
        };
        if (!matches(thread.root) && std::none_of(thread.replies.begin(), thread.replies.end(), matches)) return false;
    }
    return true;
}

QList<ReviewModel::Thread> ReviewModel::threads() const {
    QList<Thread> shown;
    for (const Thread &thread : m_threads)
        if (passes(thread)) shown << thread;
    return shown;
}

const ReviewModel::Thread *ReviewModel::thread(const QString &id) const {
    const auto found = m_threadIndex.constFind(id);
    return found == m_threadIndex.constEnd() ? nullptr : &m_threads.at(*found);
}

const ReviewModel::Thread *ReviewModel::threadOf(const QString &id) const {
    if (const Thread *direct = thread(id)) return direct;
    const auto root = m_rootOf.constFind(id);
    if (root != m_rootOf.constEnd()) return thread(*root);
    if (const Proposal *entry = proposal(id)) return thread(entry->threadId);
    return nullptr;
}

const ReviewModel::Comment *ReviewModel::comment(const QString &id) const {
    const Thread *owner = threadOf(id);
    if (!owner) return nullptr;
    if (owner->root.id == id) return &owner->root;
    for (const Comment &reply : owner->replies)
        if (reply.id == id) return &reply;
    return nullptr;
}

QStringList ReviewModel::commentIds() const {
    QStringList ids;
    for (const Comment &comment : m_comments) ids << comment.id;
    return ids;
}

const ReviewModel::Proposal *ReviewModel::proposal(const QString &id) const {
    const auto found = m_proposalIndex.constFind(id);
    return found == m_proposalIndex.constEnd() ? nullptr : &m_proposals.at(*found);
}

QList<ReviewModel::Proposal> ReviewModel::openProposals() const {
    QList<Proposal> open;
    for (const QString &id : m_pinOrder)
        if (const Proposal *entry = proposal(id)) open << *entry;
    return open;
}

QString ReviewModel::pinnedProposal() const { return m_pinOrder.isEmpty() ? QString() : m_pinOrder.first(); }

bool ReviewModel::isPinned(const QString &proposalId) const {
    const Proposal *entry = proposal(proposalId);
    return entry && !m_pinOrder.isEmpty() && entry->id == m_pinOrder.first();
}

void ReviewModel::setCurrentThread(const QString &id) {
    QString target;
    if (!id.isEmpty()) {
        const Thread *owner = threadOf(id);
        if (!owner) return;
        target = owner->id;
    }
    if (target == m_current) return;
    m_current = target;
    emit currentChanged(m_current);
}

QString ReviewModel::threadAt(double seconds, double slack) const {
    QString best;
    double bestDistance = std::numeric_limits<double>::infinity();
    for (const Thread &thread : m_threads) {
        const Comment &root = thread.root;
        if (thread.origin != Origin::Record || !root.hasAt()) continue;
        const double distance = seconds < root.at ? root.at - seconds : seconds > root.end() ? seconds - root.end() : 0.0;
        if (distance > slack + kTimeEpsilon || distance >= bestDistance) continue;
        best = thread.id;
        bestDistance = distance;
    }
    return best;
}

QList<ReviewModel::LaneMarker> ReviewModel::laneMarkers() const {
    QList<const Thread *> timed;
    for (const Thread &thread : m_threads)
        if (thread.origin == Origin::Record && thread.root.hasAt()) timed << &thread;
    std::stable_sort(timed.begin(), timed.end(), [](const Thread *a, const Thread *b) {
        if (a->root.at != b->root.at) return a->root.at < b->root.at;
        return createdBefore(a->root, b->root);
    });
    QList<LaneMarker> markers;
    for (const Thread *thread : std::as_const(timed)) {
        const Comment &root = thread->root;
        LaneMarker marker;
        marker.id = thread->id;
        marker.at = root.at;
        marker.end = root.end();
        marker.ranged = root.ranged();
        marker.kind = root.author.kind;
        marker.initials = initials(root.author);
        marker.glyph = root.author.isKerf();
        marker.circle = root.author.isPerson();
        marker.name = displayName(root.author);
        marker.role = root.author.role;
        marker.body = root.body;
        marker.resolved = thread->resolved();
        marker.current = isCurrent(thread->id);
        marker.own = isYou(root.author);
        markers << marker;
    }
    return markers;
}

QList<ReviewModel::MarkerStack> ReviewModel::markerStacks(double widthPx, double durationSec) const {
    const auto x = [&](double at) {
        if (widthPx <= 2 * kLaneInset) return widthPx / 2;
        const double raw = durationSec > 0.0 ? at / durationSec * widthPx : 0.0;
        return std::clamp(raw, kLaneInset, widthPx - kLaneInset);
    };
    QList<MarkerStack> stacks;
    double last = 0.0;
    for (const LaneMarker &marker : laneMarkers()) {
        const double at = x(marker.at);
        // Within 20 px of the group's last marker joins the group, as the
        // lane draws them: a chain of near marks is one stack.
        if (!stacks.isEmpty() && at - last < kStackDistance) {
            stacks.last().markers << marker;
        } else {
            MarkerStack stack;
            stack.x = at;
            stack.markers << marker;
            stacks << stack;
        }
        last = at;
    }
    return stacks;
}

QList<ReviewModel::TagCount> ReviewModel::tags() const {
    QMap<QString, TagCount> byKey;
    int flagged = 0;
    for (const Thread &thread : m_threads) {
        if (thread.flagged) ++flagged;
        for (const QString &tag : thread.tags) {
            TagCount &entry = byKey[tag.toLower()];
            if (entry.tag.isEmpty()) entry.tag = tag;
            ++entry.count;
        }
    }
    QList<TagCount> out = byKey.values();
    if (flagged > 0) out << TagCount{QString::fromLatin1(kBlockingTag), flagged, true};
    return out;
}

QString ReviewModel::identityEmail() const { return m_identity.value(QStringLiteral("email")).toString().trimmed(); }

QString ReviewModel::identityName() const { return m_identity.value(QStringLiteral("name")).toString(); }

bool ReviewModel::guestMode() const {
    return !m_viewAs.isEmpty() && m_viewAs.compare(identityEmail(), Qt::CaseInsensitive) != 0;
}

QString ReviewModel::viewAsName() const {
    if (m_viewAs.isEmpty()) return {};
    for (const Comment &comment : m_comments)
        if (comment.author.isPerson() && comment.author.id.compare(m_viewAs, Qt::CaseInsensitive) == 0 &&
            !comment.author.name.isEmpty())
            return comment.author.name;
    return m_viewAs;
}

QList<ReviewModel::Person> ReviewModel::people() const {
    QList<Person> out;
    QSet<QString> seen;
    for (const Comment &comment : m_comments) {
        const Author &author = comment.author;
        if (!author.isPerson() || author.id.isEmpty() || isYou(author)) continue;
        const QString key = author.id.toLower();
        if (seen.contains(key)) continue;
        seen.insert(key);
        out << Person{author.id, author.name.isEmpty() ? author.id : author.name};
    }
    std::stable_sort(out.begin(), out.end(), [](const Person &a, const Person &b) {
        return QString::compare(a.name, b.name, Qt::CaseInsensitive) < 0;
    });
    return out;
}

bool ReviewModel::isYou(const Author &author) const {
    const QString email = identityEmail();
    return author.isPerson() && !email.isEmpty() && author.id.trimmed().compare(email, Qt::CaseInsensitive) == 0;
}

bool ReviewModel::isOwn(const QJsonObject &comment) const { return isYou(readAuthor(comment.value(QStringLiteral("author")))); }

QString ReviewModel::displayName(const Author &author) const {
    return isYou(author) ? QStringLiteral("You") : author.name;
}

} // namespace kerf
