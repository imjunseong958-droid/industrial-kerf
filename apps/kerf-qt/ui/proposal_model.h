#pragma once

// The proposals of the state snapshot: what Kerf (or an agent over MCP) has
// staged and the person has not yet accepted or rejected, each with every
// version it has had and the clips it changes against the program. A pure
// model with no widgets; the conversation, the timeline, the stage and the
// toolbar read the same instance (EditorBus::proposals()).
//
// Inputs, all bridge data:
//   state {snapshot} → setSnapshot(snapshot), or setTimeline() and
//                      setProposals(proposals, proposalTimelines)
//   clock / seek     → setPlayhead(seconds), which orders the open ones
// applyEvent(type, event) routes the state event.
//
// Lifetime: what the model hands out by pointer or reference (proposal(),
// proposals()) points into a list every rebuild replaces. Such a pointer is
// valid only until the next changed(); keep ids across it.

#include <QDateTime>
#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QList>
#include <QObject>
#include <QString>
#include <QStringList>

namespace kerf {

class ProposalModel final : public QObject {
    Q_OBJECT
public:
    // Who staged a proposal: kerf (Kerf itself), mcp (an agent over MCP:
    // Claude Code, Cursor, Codex), person.
    struct Author {
        QString kind;
        QString name;
        QString id;   // a person's email, an agent's client id
        QString role; // shown beside the name: "MCP · via Kerf"
        QString initials; // when the record carries them ("Cx"); initials() falls back to the name
        bool isPerson() const { return kind == QLatin1String("person"); }
        bool isAgent() const { return kind == QLatin1String("mcp"); }
        bool isKerf() const { return kind == QLatin1String("kerf") || kind == QLatin1String("qc"); }
        bool operator==(const Author &other) const {
            return kind == other.kind && name == other.name && id == other.id && role == other.role &&
                   initials == other.initials;
        }
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

    // A proposal of the snapshot, with the clips it changes.
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

        QString summary; // the script's first change, in words
        QList<ClipChange> changes;
        bool hasRange = false;
        double start = 0.0, end = 0.0;

        bool isOpen() const { return status == QLatin1String("pending"); }
        // Seconds from `playhead` to the range: 0 inside, infinite with no range.
        double distanceFrom(double playhead) const;
        bool hasVersion(const QString &proposalId) const;
    };

    explicit ProposalModel(QObject *parent = nullptr);

    // Inputs.
    void setProposals(const QJsonArray &proposals, const QJsonObject &proposalTimelines);
    // The program timeline proposal timelines are compared with.
    void setTimeline(const QJsonObject &timeline);
    // The snapshot's timeline, proposals and proposalTimelines at once.
    void setSnapshot(const QJsonObject &snapshot);
    void setPlayhead(double seconds);
    // The state event; false for any other.
    bool applyEvent(const QString &type, const QJsonObject &event);

    // Every proposal, in the snapshot's order (newest first).
    const QList<Proposal> &proposals() const { return m_proposals; }
    // By any of its versions' ids.
    const Proposal *proposal(const QString &id) const;
    // Pending proposals in pin order: the nearest to the playhead, then the
    // snapshot's order.
    QList<Proposal> openProposals() const;
    int openCount() const { return int(m_pinOrder.size()); }
    QString pinnedProposal() const;
    bool isPinned(const QString &proposalId) const;
    double playhead() const { return m_playhead; }

    static QString initials(const Author &author);
    static Author readAuthor(const QJsonValue &value);
    // The clips a proposal timeline changes against the program's.
    static QList<ClipChange> diffTimelines(const QJsonObject &program, const QJsonObject &proposed);

signals:
    // Proposals or pin order changed.
    void changed();

private:
    void rebuild();
    QStringList pinOrder() const;

    QJsonArray m_proposalJson;
    QJsonObject m_proposalTimelines;
    QJsonObject m_timeline;
    double m_playhead = 0.0;

    QList<Proposal> m_proposals;
    QHash<QString, int> m_proposalIndex; // any version id → index
    QStringList m_pinOrder;
};

} // namespace kerf
