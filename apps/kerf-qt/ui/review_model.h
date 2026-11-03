#pragma once

// The review: comment threads from kerfd's review log, the proposals of the
// state snapshot, and who is looking at them. A pure model with no widgets;
// the Comments panel, the scrub lane, the ruler, the rail badge and the
// Deliver gate all read the same instance (EditorBus::review()).
//
// Inputs, all bridge data (plan §3):
//   comments {items}        → setComments(items)
//   state {snapshot}        → setSnapshot(snapshot), or setTimeline() and
//                             setProposals(proposals, proposalTimelines)
//   collab {status: {user}} → setIdentity(user)
//   clock / seek            → setPlayhead(seconds)
//   View as … (More menu)   → setViewAs(person id, or empty)
// applyEvent(type, event) routes the three bridge events above.
//
// Nothing here is invented: a thread is a root comment and its replies, a
// pending proposal no comment links to gets a thread of its own written
// from the proposal (author, script), and the Kerf welcome row is a thread
// only while the panel shows it (setWelcomeShown).
//
// Lifetime: everything the model hands out by pointer or reference
// (thread(), threadOf(), comment(), proposal(), allThreads(), proposals())
// points into lists every rebuild replaces: any comments, state, collab or
// welcome change. Such a pointer is valid only until the next changed();
// keep ids across it and look them up again.

#include <QDateTime>
#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QList>
#include <QObject>
#include <QString>
#include <QStringList>

#include <cmath>
#include <limits>

namespace kerf {

class ReviewModel final : public QObject {
    Q_OBJECT
public:
    // Who wrote a comment or a proposal: kerf (Kerf itself), qc (checks),
    // mcp (an agent over MCP: Claude Code, Cursor, Codex), person.
    struct Author {
        QString kind;
        QString name;
        QString id;   // a person's email, an agent's client id
        QString role; // shown beside the name: "director", "MCP · via Kerf"
        QString initials; // when the record carries them ("Cx"); initials() falls back to the name
        bool isPerson() const { return kind == QLatin1String("person"); }
        bool isAgent() const { return kind == QLatin1String("mcp"); }
        // Kerf and QC: drawn with a glyph, filtered under "Kerf".
        bool isKerf() const { return kind == QLatin1String("kerf") || kind == QLatin1String("qc"); }
        bool operator==(const Author &other) const {
            return kind == other.kind && name == other.name && id == other.id && role == other.role &&
                   initials == other.initials;
        }
    };

    // One record of `comments.items`, read (plan §3 comment record).
    struct Comment {
        QString id;
        QString parent;
        Author author;
        double at = std::numeric_limits<double>::quiet_NaN(); // seconds on the program; NaN when it has none
        double duration = 0.0;
        int frame = -1;
        QString body;
        QStringList tags; // without '#'; the literal tag "blocking" is folded into `blocking`
        bool blocking = false;
        QJsonArray drawing;
        QString proposal; // the proposal id (any version) it carries
        QStringList steps;
        bool error = false;
        bool resolved = false;
        bool hasResolvedBy = false;
        Author resolvedBy;
        qint64 resolvedRevision = -1;
        QString qcKey;
        QDateTime created;
        QString createdText; // as kerfd wrote it (RFC 3339, microseconds): breaks ties QDateTime's ms cannot
        qint64 revision = -1;
        QDateTime edited;
        QJsonObject json; // the record as it came, for anything not read above

        bool hasAt() const { return !std::isnan(at); }
        bool ranged() const { return hasAt() && duration > 0.0; }
        double end() const { return hasAt() ? at + duration : at; }
        bool hasDrawing() const { return !drawing.isEmpty(); }
    };

    enum class Origin {
        Record,   // a root comment of the review log
        Proposal, // written from a pending proposal no comment links to
        Welcome,  // the Kerf welcome row (no record)
    };

    struct Thread {
        QString id; // the root comment's id; "proposal:<id>" or kWelcomeId otherwise
        Origin origin = Origin::Record;
        Comment root;
        QList<Comment> replies; // oldest first
        // The proposals the thread's comments carry, by their current id, in
        // the snapshot's order (newest first); settled ones included.
        QStringList proposals;
        // The first of those still pending, or empty.
        QString openProposal;
        // Settled by accepting a linked proposal. kerfd's accept resolves
        // the comments carrying the proposal (any version), not the root, at
        // the revision the proposal made; a thread whose linking comments are
        // all resolved, one of them at the accepted proposal's applied
        // revision, is settled by it ("Accepted in rev N"). The latest such
        // proposal, or empty and -1.
        QString acceptedProposal;
        qint64 acceptedRevision = -1;
        // Not settled: the root is not resolved, and no accepted proposal
        // settled it unless another is still pending. A synthesised pending
        // proposal thread is open; the welcome is not.
        bool open = false;
        bool blocking = false; // open, and a comment in it is blocking and not resolved
        bool flagged = false;  // a comment in it is marked blocking, resolved or not (the Blocking tag)
        bool hasDrawing = false;
        QStringList tags; // the user tags of all its comments, each once, in first-seen order

        int size() const { return 1 + int(replies.size()); }
        // A record thread that is settled, by its root's resolve or an accept.
        bool resolved() const { return origin == Origin::Record && !open; }
    };

    struct Counts {
        int all = 0;       // threads
        int open = 0;      // open threads
        int proposals = 0; // pending proposals
        int blocking = 0;  // open threads with an open blocking comment
        bool operator==(const Counts &o) const {
            return all == o.all && open == o.open && proposals == o.proposals && blocking == o.blocking;
        }
        bool operator!=(const Counts &o) const { return !(*this == o); }
    };

    // A clip (or a transition) a proposal changes, from its proposal
    // timeline against the program's: start, end (trim), effects,
    // properties, enabled, speed or reverse, link, track, or added or removed.
    struct ClipChange {
        QString clip; // the clip's id, or the transition's when `transition`
        bool transition = false; // a track transition: from/to/fx/align in `properties`
        QString track; // where it ends up (or was, when removed)
        QString name;
        bool added = false;
        bool removed = false;
        bool moved = false;   // start changed
        bool trimmed = false; // duration changed
        bool effects = false; // effect stack changed
        bool properties = false; // clip properties, generator parameters or caption text changed
        bool retracked = false;  // moved to another track
        bool toggled = false;    // enabled or disabled
        bool retimed = false;    // speed or reverse changed
        bool relinked = false;   // its link group changed
        double oldStart = 0.0, oldEnd = 0.0; // unset when added
        double newStart = 0.0, newEnd = 0.0; // unset when removed
        double shift() const { return added || removed ? 0.0 : newStart - oldStart; }
    };

    struct Version {
        int version = 1;
        QString proposalId;
        QString script;
        QString documentDiff;
    };

    // A proposal of the snapshot, with what the review adds to it.
    struct Proposal {
        QString id; // the current version's proposalId (the accepted one, once settled)
        QString status; // pending | accepted | rejected
        bool stale = false;
        qint64 baseRevision = -1;
        qint64 appliedRevision = -1;
        QString script;
        QString source;
        QString documentDiff;
        Author author;
        int version = 1;
        QString supersedes;
        QList<Version> versions; // oldest first, the current one included
        QDateTime staged;
        QJsonObject json;

        QString threadId;       // the thread it belongs to
        bool synthesised = false; // that thread was written from the proposal
        bool blocking = false;  // its thread is blocking
        QString summary;        // the linking comment's first line, else the script's
        QList<ClipChange> changes;
        bool hasRange = false;
        double start = 0.0, end = 0.0;

        bool isOpen() const { return status == QLatin1String("pending"); }
        // Seconds from `playhead` to the range: 0 inside, infinite with no range.
        double distanceFrom(double playhead) const;
        bool hasVersion(const QString &proposalId) const;
    };

    // A root comment with a time, as the scrub lane and the ruler draw it.
    struct LaneMarker {
        QString id;
        double at = 0.0;
        double end = 0.0; // at + duration
        bool ranged = false;
        QString kind;     // the author kind: person | mcp | kerf | qc
        QString initials; // "MP", "Cu"; "K" and "Q" for the glyph kinds
        bool glyph = false; // kerf and qc draw a glyph instead of initials
        bool circle = false; // people are circles, agents and checks rounded squares
        QString name, role, body;
        bool resolved = false;
        bool current = false;
        bool own = false;
    };

    // Markers that fall within 20 px of each other at a lane width: one
    // count stack when it holds more than one. `x` is where it is drawn
    // (its first marker's, clamped 10 px inside the lane).
    struct MarkerStack {
        double x = 0.0;
        QList<LaneMarker> markers;
        bool isStack() const { return markers.size() > 1; }
    };

    struct TagCount {
        QString tag; // kBlockingTag for the Blocking entry
        int count = 0;
        bool blocking = false;
    };

    struct Person {
        QString id; // email
        QString name;
    };

    enum class Chip { All, Open, Proposals };
    enum class From { Anyone, Kerf, Agents, People };
    enum class Sort { Timecode, Newest, OpenFirst, Commenter };

    static constexpr const char *kWelcomeId = "kerf:welcome";
    static constexpr const char *kProposalThreadPrefix = "proposal:";
    // The tag filter that selects threads marked blocking.
    static constexpr const char *kBlockingTag = "blocking";
    static constexpr double kStackDistance = 20.0;
    static constexpr double kLaneInset = 10.0;

    explicit ReviewModel(QObject *parent = nullptr);

    // Inputs.
    void setComments(const QJsonArray &items);
    void setProposals(const QJsonArray &proposals, const QJsonObject &proposalTimelines);
    // The program timeline proposal timelines are compared with.
    void setTimeline(const QJsonObject &timeline);
    // The snapshot's timeline, proposals and proposalTimelines at once.
    void setSnapshot(const QJsonObject &snapshot);
    // collab.status.user: {name, email}; null or empty clears it.
    void setIdentity(const QJsonObject &user);
    void setPlayhead(double seconds);
    void setViewAs(const QString &personId);
    void setWelcomeShown(bool shown);
    // comments, state and collab events; false for any other.
    bool applyEvent(const QString &type, const QJsonObject &event);

    // Filters (each emits changed() when it changes what is shown).
    void setChip(Chip chip);
    void setFrom(From from);
    void setHideResolved(bool hide);
    void setWithDrawings(bool with);
    // One tag at a time; empty clears it.
    void setTag(const QString &tag);
    // The same tag again clears it, as clicking a tag in a row does.
    void toggleTag(const QString &tag);
    void setSearch(const QString &text);
    void setSort(Sort sort);
    void clearFilters();

    Chip chip() const { return m_chip; }
    From from() const { return m_from; }
    bool hideResolved() const { return m_hideResolved; }
    bool withDrawings() const { return m_withDrawings; }
    QString tag() const { return m_tag; }
    QString search() const { return m_search; }
    Sort sort() const { return m_sort; }
    // Whether anything but the chip and the sort narrows the list.
    bool filtering() const;
    static QString sortName(Sort sort);

    // Outputs.
    // The threads shown, filtered and sorted; the welcome is always first.
    QList<Thread> threads() const;
    // Every thread, sorted, unfiltered.
    const QList<Thread> &allThreads() const { return m_threads; }
    // The pointers below are valid only until the next changed() (see the top).
    bool passes(const Thread &thread) const;
    Counts counts() const { return m_counts; }
    const Thread *thread(const QString &id) const;
    // The thread holding a comment (a root or a reply), or a proposal.
    const Thread *threadOf(const QString &commentOrProposalId) const;
    const Comment *comment(const QString &id) const;
    QStringList commentIds() const;

    // Every proposal, in the snapshot's order (newest first).
    const QList<Proposal> &proposals() const { return m_proposals; }
    // By any of its versions' ids.
    const Proposal *proposal(const QString &id) const;
    // Pending proposals in pin order: blocking first, then the nearest to
    // the playhead, then the snapshot's order.
    QList<Proposal> openProposals() const;
    QString pinnedProposal() const;
    bool isPinned(const QString &proposalId) const;

    QString currentThread() const { return m_current; }
    // A root, a reply (its thread), or a proposal (its thread); empty clears.
    void setCurrentThread(const QString &id);
    bool isCurrent(const QString &threadId) const { return !m_current.isEmpty() && m_current == threadId; }
    // The root whose mark holds `seconds` (its range, or within `slack` of
    // its time), the nearest if several; empty when none.
    QString threadAt(double seconds, double slack = 0.0) const;

    // One per root comment with a time, by time.
    QList<LaneMarker> laneMarkers() const;
    QList<MarkerStack> markerStacks(double widthPx, double durationSec) const;
    // User tags by name, then Blocking, each with how many threads carry it.
    QList<TagCount> tags() const;

    double playhead() const { return m_playhead; }
    QJsonObject identity() const { return m_identity; }
    QString identityEmail() const;
    QString identityName() const;
    QString viewAs() const { return m_viewAs; }
    // The person viewed as, while it is not you.
    bool guestMode() const;
    QString viewAsName() const;
    // The people who commented, without you, by name.
    QList<Person> people() const;
    // A person whose id is the identity's email: "You".
    bool isYou(const Author &author) const;
    bool isOwn(const Comment &comment) const { return isYou(comment.author); }
    bool isOwn(const QJsonObject &comment) const;
    // "You" for your own comments, else the name.
    QString displayName(const Author &author) const;
    static QString initials(const Author &author);
    static Comment readComment(const QJsonObject &record);
    static Author readAuthor(const QJsonValue &value);
    // The clips a proposal timeline changes against the program's.
    static QList<ClipChange> diffTimelines(const QJsonObject &program, const QJsonObject &proposed);

signals:
    // Threads, proposals, filters, sort or pin order changed.
    void changed();
    void countsChanged();
    void currentChanged(const QString &threadId);

private:
    void rebuild();
    void refilter();
    QStringList pinOrder() const;
    double sortTime(const Thread &thread) const;
    void sortThreads(QList<Thread> &threads) const;

    QJsonArray m_commentJson;
    QList<Comment> m_comments;
    QJsonArray m_proposalJson;
    QJsonObject m_proposalTimelines;
    QJsonObject m_timeline;
    QJsonObject m_identity;
    QString m_viewAs;
    double m_playhead = 0.0;
    bool m_welcome = false;

    Chip m_chip = Chip::All;
    From m_from = From::Anyone;
    bool m_hideResolved = false;
    bool m_withDrawings = false;
    QString m_tag;
    QString m_search;
    Sort m_sort = Sort::Timecode;

    QList<Thread> m_threads;
    QHash<QString, int> m_threadIndex;  // thread id → index
    QHash<QString, QString> m_rootOf;   // comment id → thread id
    QList<Proposal> m_proposals;
    QHash<QString, int> m_proposalIndex; // any version id → index
    Counts m_counts;
    QString m_current;
    QStringList m_pinOrder;
};

} // namespace kerf
