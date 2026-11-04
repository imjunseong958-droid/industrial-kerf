#pragma once

// The Effects and titles sheet (mockup-spec 8), 304 wide: a
// 28-point strip with the segmented Effects | Titles | Presets and Search
// (⌘⇧F), over the page chosen:
//
//   Effects  the catalog's effects (EffectsPanel), previewed on the selected
//            clip or the program's clip under the playhead
//   Titles   the catalog's generators (an EffectsPanel in Titles mode),
//            previewed alone on black
//   Presets  the project's preset library (PresetsPanel)
//
// Each page has its own category row and an honest empty state (no catalog
// or no bridge). BinWiring feeds the pages from the engine.

#include <QPointer>
#include <QString>
#include <QWidget>

class QAbstractButton;
class QAction;
class QHBoxLayout;
class QStackedWidget;

namespace kerf {

class EditorBus;
class EffectsPanel;
class PresetsPanel;

namespace ui {
class IconButton;
class SegmentedControl;
} // namespace ui

class BrowserPanel final : public QWidget {
    Q_OBJECT
public:
    enum class Page { Effects, Titles, Presets };

    explicit BrowserPanel(QWidget *parent, EditorBus *bus);

    void setEffects(EffectsPanel *effects);
    EffectsPanel *effects() const { return m_effects; }
    void setTitles(EffectsPanel *titles);
    EffectsPanel *titles() const { return m_titles; }
    void setPresets(PresetsPanel *presets);
    PresetsPanel *presets() const { return m_presets; }

    Page page() const { return m_page; }
    // A page not available is refused.
    void setPage(Page page);
    // Every page is available (Titles since W2).
    static bool pageAvailable(Page page);
    // "effects", "titles", "presets"; and back (Effects for anything else).
    static QString pageId(Page page);
    static Page pageFromId(const QString &id);
    QAbstractButton *pageButton(Page page) const;
    ui::SegmentedControl *pages() const { return m_pages; }
    QAbstractButton *searchButton() const;
    QWidget *currentPage() const;
    // Search (⌘⇧F): the search field of the Effects page, or of Titles
    // when it is in front.
    void focusSearch();
    // The registry's browser.search (⌘⇧F): its key works while the browser
    // is on screen and the Search button's tip names it. The window runs it.
    void setSearchAction(QAction *action);
    // A control at the end of the strip (the sheet's close button).
    void addToStrip(QWidget *widget);

signals:
    void pageChanged(BrowserPanel::Page page);

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    void showCurrent();
    void updateSearchTip();

    EditorBus *m_bus = nullptr;
    QHBoxLayout *m_stripLayout = nullptr;
    EffectsPanel *m_effects = nullptr;
    EffectsPanel *m_titles = nullptr;
    PresetsPanel *m_presets = nullptr;
    QStackedWidget *m_stack = nullptr;
    // What a page shows before its panel is set.
    QWidget *m_placeholder = nullptr;
    ui::SegmentedControl *m_pages = nullptr;
    ui::IconButton *m_search = nullptr;
    QPointer<QAction> m_searchAction;
    Page m_page = Page::Effects;
};

} // namespace kerf
