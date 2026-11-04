#pragma once

// The Inspector sheet (mockup-spec 5): a 28-point strip with the segmented
// Inspector | Source and the kinds Video, Audio and Info, over the pages it
// stacks: the Inspector (InspectorPanel), the compact Source monitor, and
// while the Deliver tray is open and Video is chosen, the Deliver pane.
//
// The kinds are the Inspector's panes (InspectorPanel::setPane): Video,
// Audio and Info. Choosing a kind turns the Inspector page back on; a tray
// opening or closing goes back to the Inspector page (spec 10).

#include <QWidget>

class QAbstractButton;
class QHBoxLayout;
class QStackedWidget;

namespace kerf {

class DeliverPane;
class EditorBus;
namespace ui {
class SegmentedControl;
}
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
    // Shows the kind's pane of the Inspector.
    void setKind(Kind kind);
    static bool kindAvailable(Kind kind);
    static QString kindId(Kind kind);
    static Kind kindFromId(const QString &id);
    // The Deliver pane stands in for the Inspector (the Deliver tray, Video).
    bool deliverShown() const;
    QAbstractButton *pageButton(Page page) const;
    QAbstractButton *kindButton(Kind kind) const;
    // The widget the column shows now.
    QWidget *currentPage() const;
    // A control at the end of the strip (the sheet's close button).
    void addToStrip(QWidget *widget);

signals:
    void pageChanged(InspectorColumn::Page page);
    void kindChanged(InspectorColumn::Kind kind);

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    void showCurrent();

    EditorBus *m_bus = nullptr;
    QHBoxLayout *m_stripLayout = nullptr;
    InspectorPanel *m_inspector = nullptr;
    SourceMonitorPanel *m_source = nullptr;
    DeliverPane *m_deliver = nullptr;
    QStackedWidget *m_stack = nullptr;
    QWidget *m_empty = nullptr;
    ui::SegmentedControl *m_pages = nullptr;
    ui::SegmentedControl *m_kinds = nullptr;
    QAbstractButton *m_pageButtons[2] = {};
    QAbstractButton *m_kindButtons[3] = {};
    Page m_page = Page::Inspector;
    Kind m_kind = Kind::Video;
};

} // namespace kerf
