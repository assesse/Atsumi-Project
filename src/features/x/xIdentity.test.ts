import {expect,it} from 'vitest';
import script from '../../../src-tauri/src/x_identity.js?raw';
function identity(html:string,path='/home',hostname='x.com'){
  const doc=new DOMParser().parseFromString(html,'text/html'),win={top:null as unknown};win.top=win;
  return new Function('window','document','location','URL',`return (${script.replace('__ATSUMI_X_NONCE__','"fixture"')});`)(win,doc,{protocol:'https:',hostname,origin:'https://'+hostname,pathname:path},URL);
}
const signed='<a data-testid="AppTabBar_Profile_Link" href="/my_account"></a><button data-testid="SideNav_AccountSwitcher_Button"></button>';
it('requires both rendered account controls and never accepts login or challenge pages',()=>{
  expect(identity(signed)).toEqual({nonce:'fixture',handle:'my_account'});
  expect(identity(signed,'/i/flow/login')).toEqual({nonce:'fixture'});
  expect(identity(signed,'/i/flow/challenge')).toEqual({nonce:'fixture'});
  expect(identity(signed.replace('<button data-testid="SideNav_AccountSwitcher_Button"></button>',''))).toEqual({nonce:'fixture'});
  expect(identity(signed.replace('href="/my_account"','href="https://evil.test/my_account"'))).toEqual({nonce:'fixture'});
  expect(identity(signed,'/home','evil.test')).toEqual({nonce:'fixture'});
});
it('does not automate sign-in or access credentials',()=>{expect(script).not.toMatch(/\.click\(|\.value|cookie|localStorage|sessionStorage|fetch\(|XMLHttpRequest|__INITIAL_STATE__|graphql/);});
