#pragma once

// The review region's wiring: the Comments panel (the agent chat), its
// review model, proposals, Ask and next steps. A friend of MainWindow that
// holds the region's own state, so the region changes the window's
// behaviour only here and in main_window_review.cpp.
//
// It owns the ReviewModel every region reads (EditorBus::review()), feeds
// it the bridge's comments, state and collab events, and turns what the
// panel asks for into bridge commands (plan §3):
//   post          → agent {text, focus, comment: {at?, parent?, drawing?}} or comment_add
//   resolve       → comment_set {id, resolved}
//   accept/reject → accept/reject {proposalId, revision}
//   Preview       → preview {proposalId, start, duration} / preview_exit
//   colour chip   → effect_previews {clip, at, effects: [{fx, params, key}]}
//   row menu      → edit add-marker, comment_set {id, body | deleted}
//   More menu     → export_comments {format, includeResolved, path?}
//
// The member functions that reach into MainWindow are defined in
// main_window_review.cpp, which the window's executables compile.

#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QObject>
#include <QPointer>
#include <QString>

#include <cmath>
#include <limits>

namespace kerf {

class ChatPanel;
class EditorBus;
class MainWindow;
class ReviewModel;

class ReviewWiring final : public QObject {
    Q_OBJECT
public:
    ReviewWiring(MainWindow *window, EditorBus *bus);
    ~ReviewWiring() override;
    MainWindow *window() const { return m_window; }
    EditorBus *bus() const { return m_bus; }
    // The review model, set on the bus once the panel exists.
    ReviewModel *model() const { return m_model; }
    ChatPanel *panel() const;

    // Connects the panel, the model and the bus (MainWindow::buildReview).
    void attach(ChatPanel *panel);

    // What the composer holds, posted: an agent turn when it is for Kerf
    // (it mentions @Kerf, or no one), else a note (comment_add).
    bool post();
    // An agent request, kept in the review log as the reader's comment
    // (`comment`: at, duration, parent, drawing) in `threadId`.
    bool sendTurn(const QString &text, const QJsonObject &focus, const QString &focusLabel, const QJsonObject &comment,
                  const QString &threadId);
    // The comment the agent request records for a focus (the Ask popover):
    // its range on the program, or nothing for the whole sequence.
    QJsonObject commentForFocus(const QJsonObject &focus) const;

    bool resolveThread(const QString &threadId, bool resolve);
    bool accept(const QString &proposalId);
    bool reject(const QString &proposalId);
    bool preview(const QString &proposalId, bool on);
    bool previewRange(const QString &proposalId, double start, double duration);
    bool requestColour(const QString &versionId);
    bool exportComments(const QString &format, const QString &path = QString());
    bool convertToMarker(const QString &commentId);
    bool deleteComment(const QString &commentId);
    bool editComment(const QString &commentId, const QString &body);
    // kerf://<project>/rev<N>@TC, or kerf://<project>/rev<N> for NaN.
    QString reviewLink(double seconds) const;
    // The pinned proposal is the one ⌘↩ and Esc act on (MainWindow::m_proposalId).
    void syncPinnedProposal();
    // A bridge event, after the window handled it.
    void handleEvent(const QString &type, const QJsonObject &event);

    // The last client id comment_add used (for tests).
    QString lastClientId() const { return m_lastClientId; }

private:
    QString nextClientId();

    MainWindow *m_window = nullptr;
    EditorBus *m_bus = nullptr;
    ReviewModel *m_model = nullptr;
    QPointer<ChatPanel> m_panel;
    QString m_lastClientId;
    int m_serial = 0;
};

} // namespace kerf
