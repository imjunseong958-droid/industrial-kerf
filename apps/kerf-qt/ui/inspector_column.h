#pragma once

// The Inspector column (mockup-spec 5): a 28-point strip with the segmented
// Inspector | Source and the kinds Video, Audio and Info, over the pages it
// stacks: the Inspector (InspectorPanel), the compact Source monitor, and
// in the Deliver workspace, while Video is chosen, the Deliver pane.
//
// Shell from Q-SKELETON (W1): Video is the InspectorPanel as it is; Audio
// and Info are disabled until Q-INSPECTOR builds them in W2 ("Coming in this
// release"). Q-INSPECTOR fills it in, keeping the constructor and this API.

#include <QWidget>

class QAbstractButton;
class QStackedWidget;

namespace kerf {

class DeliverPane;
class EditorBus;
class InspectorPanel;
class SourceMonitorPanel;

class InspectorColumn final : public QWidget {
    Q_OBJECT
public:
    enum class Page { Inspector, Source };
    enum class Kind { Video, Audio, Info };

    explicit InspectorColumn(QWidget *parent, EditorBus *bus);

    // The pages it shows; the column takes them as children.
    void setInspector(InspectorPanel *inspector);
    InspectorPanel *inspector() const { return m_inspector; }
    void setSourceMonitor(SourceMonitorPanel *source);
    SourceMonitorPanel *sourceMonitor() const { return m_source; }
    DeliverPane *deliverPane() const { return m_deliver; }

    Page page() const { return m_page; }
    void setPage(Page page);
    Kind kind() const { return m_kind; }
    // A kind not available yet is refused.
    void setKind(Kind kind);
    static bool kindAvailable(Kind kind);
    static QString kindId(Kind kind);
    static Kind kindFromId(const QString &id);
    // The Deliver pane stands in for the Inspector (Deliver workspace, Video).
    bool deliverShown() const;
    QAbstractButton *pageButton(Page page) const;
    QAbstractButton *kindButton(Kind kind) const;
    // The widget the column shows now.
    QWidget *currentPage() const;

signals:
    void pageChanged(InspectorColumn::Page page);
    void kindChanged(InspectorColumn::Kind kind);

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    void showCurrent();

    EditorBus *m_bus = nullptr;
    InspectorPanel *m_inspector = nullptr;
    SourceMonitorPanel *m_source = nullptr;
    DeliverPane *m_deliver = nullptr;
    QStackedWidget *m_stack = nullptr;
    QWidget *m_empty = nullptr;
    QAbstractButton *m_pageButtons[2] = {};
    QAbstractButton *m_kindButtons[3] = {};
    Page m_page = Page::Inspector;
    Kind m_kind = Kind::Video;
};

} // namespace kerf
